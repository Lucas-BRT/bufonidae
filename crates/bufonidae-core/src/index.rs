//! Local index of what is stored where, on Turso (an embedded database).
//!
//! The index is a cache: the remote listing is the source of truth. It also
//! doubles as the write-ahead journal: a placement is recorded `pending`
//! before its upload starts and `done` after it succeeds, so a crash leaves
//! an explicit trace for `reconcile` to inspect.
//!
//! Turso allows one process per database file: the CLI and the desktop app
//! cannot have the same index open at once.

use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

use turso::{Builder, Connection, params};

const SCHEMA_VERSION: i64 = 4;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("index database: {0}")]
    Db(#[from] turso::Error),
    #[error("index file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("index schema version {found} is newer than this build supports ({SCHEMA_VERSION})")]
    TooNew { found: i64 },
    #[error("index data is invalid: {0}")]
    Corrupt(String),
    #[error("could not determine the data directory")]
    NoDataDir,
    #[error(
        "the index is in use by another Bufonidae process (is the desktop app or \
         another command running?); close it and try again"
    )]
    InUse,
}

/// Confirmed or in-flight placement of a file on an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Pending,
    Done,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Done => "done",
        }
    }
}

/// A file as the index knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedFile {
    pub path: String,
    pub size: u64,
    /// Ids of the accounts holding a confirmed copy.
    pub accounts: Vec<String>,
    /// BLAKE3 of the file as it was sent, when it is known. Older entries and
    /// imported ones may have none.
    pub blake3: Option<String>,
}

/// One row of the placements table, whatever its state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub path: String,
    pub size: u64,
    pub account_id: String,
    pub state: State,
}

/// A placement whose upload started but was never confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPlacement {
    pub path: String,
    pub account_id: String,
}

/// A file that was stored and whose every copy has since disappeared. Kept so
/// that `repair` can still say it is lost, instead of the file vanishing from
/// the record as if it had never existed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LostFile {
    pub path: String,
    pub size: u64,
}

/// A persisted transfer request. Payload contains paths and operation options,
/// never credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub id: String,
    pub operation: String,
    pub payload: String,
    pub state: String,
    pub error: Option<String>,
    /// When the job was queued, in seconds since the Unix epoch.
    pub created_at: i64,
}

pub struct Index {
    conn: Connection,
    /// One connection serves every task, so an explicit transaction must not
    /// be interleaved with another task's statements.
    gate: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for Index {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Index(..)")
    }
}

/// A stored size must be a non-negative integer; anything else means the
/// database is damaged and must not be quietly read as `0`.
fn to_size(path: &str, size: i64) -> Result<u64, Error> {
    u64::try_from(size).map_err(|_| Error::Corrupt(format!("{path}: negative size {size}")))
}

impl Index {
    /// `$XDG_DATA_HOME/bufonidae/index.db`.
    pub fn default_path() -> Result<PathBuf, Error> {
        let dirs = directories::ProjectDirs::from("", "", "bufonidae").ok_or(Error::NoDataDir)?;
        Ok(dirs.data_dir().join("index.db"))
    }

    /// Opens (creating if needed) the index at `path`, mode 0600: it reveals
    /// the user's folder structure.
    pub async fn open(path: &Path) -> Result<Self, Error> {
        let io = |source| Error::Io {
            path: path.to_owned(),
            source,
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)
            .map_err(io)?;
        Self::build(&path.to_string_lossy()).await
    }

    /// A throwaway in-memory index, for tests.
    pub async fn open_memory() -> Result<Self, Error> {
        Self::build(":memory:").await
    }

    async fn build(location: &str) -> Result<Self, Error> {
        let db = Builder::new_local(location).build().await.map_err(|e| {
            // Turso allows one process per database file.
            if e.to_string().contains("locked by another process") {
                Error::InUse
            } else {
                Error::Db(e)
            }
        })?;
        let conn = db.connect()?;
        let index = Self {
            conn,
            gate: tokio::sync::Mutex::new(()),
        };
        index.migrate().await?;
        Ok(index)
    }

    async fn migrate(&self) -> Result<(), Error> {
        let mut rows = self.conn.query("PRAGMA user_version", ()).await?;
        let version: i64 = match rows.next().await? {
            Some(row) => row.get(0)?,
            None => 0,
        };
        drop(rows);
        if version > SCHEMA_VERSION {
            return Err(Error::TooNew { found: version });
        }
        if version < 1 {
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS items (
                         path TEXT PRIMARY KEY NOT NULL,
                         size INTEGER NOT NULL
                     );
                     CREATE TABLE IF NOT EXISTS placements (
                         path TEXT NOT NULL,
                         account_id TEXT NOT NULL,
                         state TEXT NOT NULL CHECK (state IN ('pending', 'done')),
                         PRIMARY KEY (path, account_id)
                     );",
                )
                .await?;
        }
        if version < 2 {
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS lost (
                         path TEXT PRIMARY KEY NOT NULL,
                         size INTEGER NOT NULL
                     );",
                )
                .await?;
        }
        if version < 3 {
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS hashes (
                         path TEXT PRIMARY KEY NOT NULL,
                         blake3 TEXT NOT NULL
                     );",
                )
                .await?;
        }
        if version < 4 {
            self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS jobs (
                     id TEXT PRIMARY KEY NOT NULL,
                     operation TEXT NOT NULL,
                     payload TEXT NOT NULL,
                     state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'interrupted', 'done', 'failed', 'cancelled')),
                     error TEXT,
                     created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                     updated_at INTEGER NOT NULL DEFAULT (unixepoch())
                 );",
            ).await?;
        }
        if version < SCHEMA_VERSION {
            // Every step above is `IF NOT EXISTS`, so a crash before this line
            // only repeats harmless work on the next start.
            self.conn
                .pragma_update("user_version", SCHEMA_VERSION)
                .await?;
        }
        Ok(())
    }

    /// Runs `work` in one transaction: all of its statements apply, or none.
    pub(crate) async fn atomic<T>(
        &self,
        work: impl AsyncFnOnce(&Connection) -> Result<T, turso::Error>,
    ) -> Result<T, Error> {
        let _turn = self.gate.lock().await;
        self.conn.execute("BEGIN", ()).await?;
        match work(&self.conn).await {
            Ok(value) => {
                self.conn.execute("COMMIT", ()).await?;
                Ok(value)
            }
            Err(error) => {
                // The original error matters more than a failed rollback.
                let _ = self.conn.execute("ROLLBACK", ()).await;
                Err(error.into())
            }
        }
    }

    /// Records the intent to upload `path` to `account_id`, before the upload.
    pub async fn mark_pending(&self, path: &str, size: u64, account_id: &str) -> Result<(), Error> {
        self.mark_pending_hashed(path, size, account_id, None).await
    }

    /// [`mark_pending`](Self::mark_pending) that also records the file's
    /// BLAKE3. A `None` keeps whatever hash is already recorded.
    pub async fn mark_pending_hashed(
        &self,
        path: &str,
        size: u64,
        account_id: &str,
        blake3: Option<&str>,
    ) -> Result<(), Error> {
        let size = i64::try_from(size)
            .map_err(|_| Error::Corrupt(format!("{path}: size {size} does not fit the index")))?;
        self.atomic(async |conn| {
            conn.execute(
                "INSERT INTO items (path, size) VALUES (?1, ?2)
                 ON CONFLICT(path) DO UPDATE SET size = excluded.size",
                params![path, size],
            )
            .await?;
            conn.execute(
                "INSERT INTO placements (path, account_id, state) VALUES (?1, ?2, ?3)
                 ON CONFLICT(path, account_id) DO UPDATE SET state = excluded.state",
                params![path, account_id, State::Pending.as_str()],
            )
            .await?;
            if let Some(hash) = blake3 {
                conn.execute(
                    "INSERT INTO hashes (path, blake3) VALUES (?1, ?2)
                     ON CONFLICT(path) DO UPDATE SET blake3 = excluded.blake3",
                    params![path, hash],
                )
                .await?;
            }
            Ok(())
        })
        .await
    }

    /// Confirms the upload started with [`mark_pending`](Self::mark_pending).
    pub async fn mark_done(&self, path: &str, account_id: &str) -> Result<(), Error> {
        self.atomic(async |conn| {
            conn.execute(
                "UPDATE placements SET state = ?3 WHERE path = ?1 AND account_id = ?2",
                params![path, account_id, State::Done.as_str()],
            )
            .await?;
            // A copy exists again: the file is no longer lost.
            conn.execute("DELETE FROM lost WHERE path = ?1", params![path])
                .await?;
            Ok(())
        })
        .await
    }

    /// Files whose path starts with `prefix`, with their confirmed accounts.
    pub async fn list(&self, prefix: &str) -> Result<Vec<IndexedFile>, Error> {
        let _turn = self.gate.lock().await;
        // `substr` compares literally; `LIKE` would treat `%` and `_` in the
        // prefix as wildcards.
        let mut rows = self
            .conn
            .query(
                "SELECT i.path, i.size, p.account_id
                 FROM items i JOIN placements p ON p.path = i.path
                 WHERE p.state = 'done' AND substr(i.path, 1, length(?1)) = ?1
                 ORDER BY i.path, p.account_id",
                params![prefix],
            )
            .await?;
        let mut files: Vec<IndexedFile> = Vec::new();
        while let Some(row) = rows.next().await? {
            let path: String = row.get(0)?;
            let size: i64 = row.get(1)?;
            let account: String = row.get(2)?;
            let size = to_size(&path, size)?;
            match files.last_mut() {
                Some(last) if last.path == path => last.accounts.push(account),
                _ => files.push(IndexedFile {
                    path,
                    size,
                    accounts: vec![account],
                    blake3: None,
                }),
            }
        }
        drop(rows);
        let hashes = self.hashes_locked(prefix).await?;
        for file in &mut files {
            file.blake3 = hashes.get(&file.path).cloned();
        }
        Ok(files)
    }

    /// Recorded hashes of the files under `prefix`, by path.
    pub async fn hashes(
        &self,
        prefix: &str,
    ) -> Result<std::collections::HashMap<String, String>, Error> {
        let _turn = self.gate.lock().await;
        self.hashes_locked(prefix).await
    }

    async fn hashes_locked(
        &self,
        prefix: &str,
    ) -> Result<std::collections::HashMap<String, String>, Error> {
        let mut rows = self
            .conn
            .query(
                "SELECT path, blake3 FROM hashes WHERE substr(path, 1, length(?1)) = ?1",
                params![prefix],
            )
            .await?;
        let mut out = std::collections::HashMap::new();
        while let Some(row) = rows.next().await? {
            out.insert(row.get::<String>(0)?, row.get::<String>(1)?);
        }
        Ok(out)
    }

    /// Every placement (pending and done) whose path starts with `prefix`.
    pub async fn placements(&self, prefix: &str) -> Result<Vec<Placement>, Error> {
        let _turn = self.gate.lock().await;
        let mut rows = self
            .conn
            .query(
                "SELECT i.path, i.size, p.account_id, p.state
                 FROM items i JOIN placements p ON p.path = i.path
                 WHERE substr(i.path, 1, length(?1)) = ?1
                 ORDER BY i.path, p.account_id",
                params![prefix],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            let path: String = row.get(0)?;
            let size = to_size(&path, row.get(1)?)?;
            let state = match row.get::<String>(3)?.as_str() {
                "done" => State::Done,
                "pending" => State::Pending,
                other => return Err(Error::Corrupt(format!("{path}: unknown state {other:?}"))),
            };
            out.push(Placement {
                size,
                path,
                account_id: row.get(2)?,
                state,
            });
        }
        Ok(out)
    }

    /// Forgets the placement of `path` on `account_id`, and the file itself
    /// when no placement is left. Touches the index only, never the remote.
    pub async fn remove_placement(&self, path: &str, account_id: &str) -> Result<(), Error> {
        self.atomic(async |conn| {
            // What was there before: a confirmed copy disappearing is a loss,
            // an upload that never finished is not.
            let mut rows = conn
                .query(
                    "SELECT p.state, i.size FROM placements p JOIN items i ON i.path = p.path
                     WHERE p.path = ?1 AND p.account_id = ?2",
                    params![path, account_id],
                )
                .await?;
            let removed = match rows.next().await? {
                Some(row) => Some((row.get::<String>(0)?, row.get::<i64>(1)?)),
                None => None,
            };
            drop(rows);

            conn.execute(
                "DELETE FROM placements WHERE path = ?1 AND account_id = ?2",
                params![path, account_id],
            )
            .await?;

            if let Some((state, size)) = removed
                && state == State::Done.as_str()
            {
                let mut rows = conn
                    .query(
                        "SELECT COUNT(*) FROM placements WHERE path = ?1 AND state = 'done'",
                        params![path],
                    )
                    .await?;
                let left: i64 = match rows.next().await? {
                    Some(row) => row.get(0)?,
                    None => 0,
                };
                drop(rows);
                if left == 0 {
                    conn.execute(
                        "INSERT INTO lost (path, size) VALUES (?1, ?2)
                         ON CONFLICT(path) DO UPDATE SET size = excluded.size",
                        params![path, size],
                    )
                    .await?;
                }
            }

            conn.execute(
                "DELETE FROM items WHERE path = ?1
                 AND NOT EXISTS (SELECT 1 FROM placements WHERE placements.path = ?1)",
                params![path],
            )
            .await?;
            // A hash is only worth keeping while the file is stored or lost.
            conn.execute(
                "DELETE FROM hashes WHERE path = ?1
                 AND NOT EXISTS (SELECT 1 FROM items WHERE items.path = ?1)
                 AND NOT EXISTS (SELECT 1 FROM lost WHERE lost.path = ?1)",
                params![path],
            )
            .await?;
            Ok(())
        })
        .await
    }

    /// Files under `prefix` that were stored and have lost every copy.
    pub async fn lost(&self, prefix: &str) -> Result<Vec<LostFile>, Error> {
        let _turn = self.gate.lock().await;
        let mut rows = self
            .conn
            .query(
                "SELECT path, size FROM lost
                 WHERE substr(path, 1, length(?1)) = ?1 ORDER BY path",
                params![prefix],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            let path: String = row.get(0)?;
            let size = to_size(&path, row.get(1)?)?;
            out.push(LostFile { path, size });
        }
        Ok(out)
    }

    /// Loads exported rows into an **empty** index, all or nothing.
    pub async fn import(
        &self,
        rows: &[Placement],
        lost: &[LostFile],
        hashes: &std::collections::HashMap<String, String>,
    ) -> Result<(), Error> {
        self.atomic(async |conn| {
            let mut found = conn
                .query(
                    "SELECT (SELECT COUNT(*) FROM items) + (SELECT COUNT(*) FROM lost)",
                    (),
                )
                .await?;
            let existing: i64 = match found.next().await? {
                Some(row) => row.get(0)?,
                None => 0,
            };
            drop(found);
            if existing > 0 {
                return Err(turso::Error::Misuse(
                    "the index is not empty; importing would mix two libraries".to_owned(),
                ));
            }
            for row in rows {
                let size = i64::try_from(row.size)
                    .map_err(|_| turso::Error::Misuse(format!("{}: size too large", row.path)))?;
                conn.execute(
                    "INSERT INTO items (path, size) VALUES (?1, ?2)
                     ON CONFLICT(path) DO NOTHING",
                    params![row.path.as_str(), size],
                )
                .await?;
                conn.execute(
                    "INSERT INTO placements (path, account_id, state) VALUES (?1, ?2, ?3)",
                    params![
                        row.path.as_str(),
                        row.account_id.as_str(),
                        row.state.as_str()
                    ],
                )
                .await?;
            }
            for (path, hash) in hashes {
                conn.execute(
                    "INSERT INTO hashes (path, blake3) VALUES (?1, ?2)",
                    params![path.as_str(), hash.as_str()],
                )
                .await?;
            }
            for file in lost {
                let size = i64::try_from(file.size)
                    .map_err(|_| turso::Error::Misuse(format!("{}: size too large", file.path)))?;
                conn.execute(
                    "INSERT INTO lost (path, size) VALUES (?1, ?2)",
                    params![file.path.as_str(), size],
                )
                .await?;
            }
            Ok(())
        })
        .await
    }

    /// Uploads that were started and never confirmed.
    pub async fn pending(&self) -> Result<Vec<PendingPlacement>, Error> {
        let _turn = self.gate.lock().await;
        let mut rows = self
            .conn
            .query(
                "SELECT path, account_id FROM placements
                 WHERE state = 'pending' ORDER BY path, account_id",
                (),
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(PendingPlacement {
                path: row.get(0)?,
                account_id: row.get(1)?,
            });
        }
        Ok(out)
    }

    /// Adds a job before any remote side effect begins.
    pub async fn enqueue_job(&self, operation: &str, payload: &str) -> Result<String, Error> {
        let _turn = self.gate.lock().await;
        let id = uuid::Uuid::new_v4().to_string();
        self.conn
            .execute(
                "INSERT INTO jobs (id, operation, payload, state) VALUES (?1, ?2, ?3, 'queued')",
                params![id.as_str(), operation, payload],
            )
            .await?;
        Ok(id)
    }

    /// Changes a job state and records an optional failure reason.
    pub async fn set_job_state(
        &self,
        id: &str,
        state: &str,
        error: Option<&str>,
    ) -> Result<(), Error> {
        if !matches!(
            state,
            "queued" | "running" | "interrupted" | "done" | "failed" | "cancelled"
        ) {
            return Err(Error::Corrupt(format!("unknown job state {state:?}")));
        }
        let _turn = self.gate.lock().await;
        self.conn
            .execute(
                "UPDATE jobs SET state = ?2, error = ?3, updated_at = unixepoch() WHERE id = ?1",
                params![id, state, error],
            )
            .await?;
        Ok(())
    }

    /// Lists persisted jobs, oldest first.
    pub async fn jobs(&self) -> Result<Vec<Job>, Error> {
        let _turn = self.gate.lock().await;
        let mut rows = self
            .conn
            .query(
                "SELECT id, operation, payload, state, error, created_at FROM jobs ORDER BY created_at, rowid",
                (),
            )
            .await?;
        let mut jobs = Vec::new();
        while let Some(row) = rows.next().await? {
            jobs.push(Job {
                id: row.get(0)?,
                operation: row.get(1)?,
                payload: row.get(2)?,
                state: row.get(3)?,
                error: row.get(4)?,
                created_at: row.get(5)?,
            });
        }
        Ok(jobs)
    }

    /// Marks jobs left active by a previous process as interrupted.
    pub async fn interrupt_active_jobs(&self) -> Result<(), Error> {
        let _turn = self.gate.lock().await;
        self.conn.execute("UPDATE jobs SET state = 'interrupted', updated_at = unixepoch() WHERE state IN ('queued', 'running')", ()).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn transfer_jobs_survive_restart_and_only_active_jobs_are_interrupted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let (running, waiting, completed);
        {
            let index = Index::open(&path).await.unwrap();
            running = index
                .enqueue_job(
                    "upload",
                    r#"{"paths":["/tmp/source"],"destination":"Docs"}"#,
                )
                .await
                .unwrap();
            waiting = index
                .enqueue_job("download", r#"["Docs/file","/tmp/out"]"#)
                .await
                .unwrap();
            completed = index.enqueue_job("download", "{}").await.unwrap();
            index
                .set_job_state(&running, "running", None)
                .await
                .unwrap();
            index.set_job_state(&completed, "done", None).await.unwrap();
        }
        let index = Index::open(&path).await.unwrap();
        index.interrupt_active_jobs().await.unwrap();
        let jobs = index.jobs().await.unwrap();
        assert_eq!(jobs.len(), 3);
        for id in [running, waiting] {
            assert_eq!(
                jobs.iter().find(|job| job.id == id).unwrap().state,
                "interrupted"
            );
        }
        assert_eq!(
            jobs.iter().find(|job| job.id == completed).unwrap().state,
            "done"
        );
        index.interrupt_active_jobs().await.unwrap();
        assert_eq!(index.jobs().await.unwrap(), jobs);
    }

    #[tokio::test]
    async fn transfer_errors_survive_and_invalid_states_are_refused() {
        let index = Index::open_memory().await.unwrap();
        let id = index.enqueue_job("upload", "{}").await.unwrap();
        index
            .set_job_state(&id, "failed", Some("network unavailable"))
            .await
            .unwrap();
        assert!(index.set_job_state(&id, "invented", None).await.is_err());
        let jobs = index.jobs().await.unwrap();
        assert_eq!(jobs[0].state, "failed");
        assert_eq!(jobs[0].error.as_deref(), Some("network unavailable"));
    }

    #[tokio::test]
    async fn pending_then_done_lifecycle() {
        let index = Index::open_memory().await.unwrap();
        index.mark_pending("Pics/a.bin", 60, "acct1").await.unwrap();

        assert!(index.list("").await.unwrap().is_empty());
        let pending = index.pending().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].account_id, "acct1");

        index.mark_done("Pics/a.bin", "acct1").await.unwrap();
        assert!(index.pending().await.unwrap().is_empty());
        let files = index.list("").await.unwrap();
        assert_eq!(
            files,
            [IndexedFile {
                path: "Pics/a.bin".into(),
                size: 60,
                accounts: vec!["acct1".into()],
                blake3: None,
            }]
        );
    }

    #[tokio::test]
    async fn list_filters_by_literal_prefix_and_groups_replicas() {
        let index = Index::open_memory().await.unwrap();
        for (path, acct) in [
            ("Pics/a.bin", "x"),
            ("Pics/a.bin", "y"),
            ("Docs/b.bin", "x"),
            ("Pic_s/c.bin", "x"),
        ] {
            index.mark_pending(path, 1, acct).await.unwrap();
            index.mark_done(path, acct).await.unwrap();
        }
        let pics = index.list("Pics/").await.unwrap();
        assert_eq!(pics.len(), 1);
        assert_eq!(pics[0].accounts, ["x", "y"]);
        // `_` is not a wildcard.
        assert_eq!(index.list("Pic_").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn persists_across_opens_with_private_mode() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data/index.db");
        {
            let index = Index::open(&path).await.unwrap();
            index.mark_pending("a", 1, "x").await.unwrap();
            index.mark_done("a", "x").await.unwrap();
        }
        let index = Index::open(&path).await.unwrap();
        assert_eq!(index.list("").await.unwrap().len(), 1);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    /// Helper for the cross-process test below, inert otherwise: opens the
    /// index named by `BUFONIDAE_HOLD_INDEX` and holds it until stdin closes.
    #[tokio::test]
    async fn hold_index_open_helper() {
        let Some(path) = std::env::var_os("BUFONIDAE_HOLD_INDEX") else {
            return;
        };
        let _index = Index::open(Path::new(&path)).await.unwrap();
        println!("ready");
        std::io::stdin().read_line(&mut String::new()).ok();
    }

    /// Turso allows one process per database file. Same-process opens are
    /// fine, so this needs a second process: the test binary relaunches
    /// itself as the holder.
    #[tokio::test]
    async fn another_process_holding_the_index_reports_in_use() {
        use std::io::{BufRead as _, BufReader};
        use std::process::{Command, Stdio};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "index::tests::hold_index_open_helper",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("BUFONIDAE_HOLD_INDEX", &path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        // Keep the reader alive until the child exits: a closed pipe would
        // make its final test output fail with a broken pipe.
        let mut lines = BufReader::new(stdout).lines().map_while(Result::ok);
        let ready = lines.by_ref().any(|line| line.contains("ready"));
        assert!(ready, "the holder process never opened the index");

        let result = Index::open(&path).await;

        drop(child.stdin.take());
        lines.for_each(drop);
        child.wait().unwrap();
        assert!(matches!(result, Err(Error::InUse)), "{result:?}");
    }

    #[tokio::test]
    async fn a_failed_transaction_leaves_nothing_behind() {
        let index = Index::open_memory().await.unwrap();
        let result: Result<(), Error> = index
            .atomic(async |conn| {
                conn.execute("INSERT INTO items (path, size) VALUES ('half', 1)", ())
                    .await?;
                // The second statement of the pair fails (no such table).
                conn.execute("INSERT INTO no_such_table VALUES (1)", ())
                    .await?;
                Ok(())
            })
            .await;
        assert!(result.is_err());
        // The first insert was rolled back, and the index is still usable.
        assert!(index.placements("").await.unwrap().is_empty());
        index.mark_pending("ok", 1, "a").await.unwrap();
        assert_eq!(index.placements("").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn losing_every_confirmed_copy_is_remembered_but_a_never_finished_upload_is_not() {
        let index = Index::open_memory().await.unwrap();
        index.mark_pending("kept", 10, "a").await.unwrap();
        index.mark_done("kept", "a").await.unwrap();
        index.mark_pending("kept", 10, "b").await.unwrap();
        index.mark_done("kept", "b").await.unwrap();
        index.mark_pending("never", 7, "a").await.unwrap();

        // One of two copies gone: still stored, not lost.
        index.remove_placement("kept", "a").await.unwrap();
        assert!(index.lost("").await.unwrap().is_empty());
        // The last copy gone: lost, with its size, and gone from the listing.
        index.remove_placement("kept", "b").await.unwrap();
        assert_eq!(
            index.lost("").await.unwrap(),
            [LostFile {
                path: "kept".into(),
                size: 10
            }]
        );
        assert!(index.list("").await.unwrap().is_empty());
        // An upload that never completed leaves no trace.
        index.remove_placement("never", "a").await.unwrap();
        assert_eq!(index.lost("").await.unwrap().len(), 1);

        // A copy turning up again clears the loss.
        index.mark_pending("kept", 10, "c").await.unwrap();
        index.mark_done("kept", "c").await.unwrap();
        assert!(index.lost("").await.unwrap().is_empty());
        assert_eq!(index.list("").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_version_one_database_is_migrated_without_losing_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        {
            // The schema exactly as version 1 created it.
            let db = Builder::new_local(&path.to_string_lossy())
                .build()
                .await
                .unwrap();
            let conn = db.connect().unwrap();
            conn.execute_batch(
                "CREATE TABLE items (path TEXT PRIMARY KEY NOT NULL, size INTEGER NOT NULL);
                 CREATE TABLE placements (
                     path TEXT NOT NULL, account_id TEXT NOT NULL,
                     state TEXT NOT NULL CHECK (state IN ('pending', 'done')),
                     PRIMARY KEY (path, account_id));
                 INSERT INTO items VALUES ('old.bin', 42);
                 INSERT INTO placements VALUES ('old.bin', 'a', 'done');",
            )
            .await
            .unwrap();
            conn.pragma_update("user_version", 1).await.unwrap();
        }
        let index = Index::open(&path).await.unwrap();
        let files = index.list("").await.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].size, 42);
        // The new table is there and works.
        index.remove_placement("old.bin", "a").await.unwrap();
        assert_eq!(index.lost("").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_damaged_size_is_an_error_not_zero() {
        let index = Index::open_memory().await.unwrap();
        index
            .atomic(async |conn| {
                conn.execute("INSERT INTO items VALUES ('bad', -5)", ())
                    .await?;
                conn.execute("INSERT INTO placements VALUES ('bad', 'a', 'done')", ())
                    .await?;
                Ok(())
            })
            .await
            .unwrap();
        assert!(matches!(index.list("").await, Err(Error::Corrupt(_))));
        assert!(matches!(index.placements("").await, Err(Error::Corrupt(_))));
    }

    #[tokio::test]
    async fn a_hash_is_kept_listed_and_cleaned_up_with_the_file() {
        let index = Index::open_memory().await.unwrap();
        let hash = "c".repeat(64);

        index
            .mark_pending_hashed("f", 5, "a", Some(&hash))
            .await
            .unwrap();
        index.mark_done("f", "a").await.unwrap();
        // A second copy recorded without a hash must not erase the hash.
        index.mark_pending("f", 5, "b").await.unwrap();
        index.mark_done("f", "b").await.unwrap();
        let files = index.list("").await.unwrap();
        assert_eq!(files[0].blake3.as_deref(), Some(hash.as_str()));

        // Losing every copy keeps the hash with the lost record.
        index.remove_placement("f", "a").await.unwrap();
        index.remove_placement("f", "b").await.unwrap();
        assert_eq!(index.hashes("").await.unwrap().len(), 1);

        // An upload that never finished leaves no hash behind.
        index
            .mark_pending_hashed("g", 1, "a", Some(&hash))
            .await
            .unwrap();
        index.remove_placement("g", "a").await.unwrap();
        assert!(!index.hashes("").await.unwrap().contains_key("g"));
    }
}
