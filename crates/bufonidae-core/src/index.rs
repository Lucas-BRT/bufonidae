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

const SCHEMA_VERSION: i64 = 1;

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

pub struct Index {
    conn: Connection,
}

impl std::fmt::Debug for Index {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Index(..)")
    }
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
        let index = Self { conn };
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
            self.conn
                .pragma_update("user_version", SCHEMA_VERSION)
                .await?;
        }
        Ok(())
    }

    /// Records the intent to upload `path` to `account_id`, before the upload.
    pub async fn mark_pending(&self, path: &str, size: u64, account_id: &str) -> Result<(), Error> {
        let size = i64::try_from(size).unwrap_or(i64::MAX);
        self.conn
            .execute(
                "INSERT INTO items (path, size) VALUES (?1, ?2)
                 ON CONFLICT(path) DO UPDATE SET size = excluded.size",
                params![path, size],
            )
            .await?;
        self.conn
            .execute(
                "INSERT INTO placements (path, account_id, state) VALUES (?1, ?2, ?3)
                 ON CONFLICT(path, account_id) DO UPDATE SET state = excluded.state",
                params![path, account_id, State::Pending.as_str()],
            )
            .await?;
        Ok(())
    }

    /// Confirms the upload started with [`mark_pending`](Self::mark_pending).
    pub async fn mark_done(&self, path: &str, account_id: &str) -> Result<(), Error> {
        self.conn
            .execute(
                "UPDATE placements SET state = ?3 WHERE path = ?1 AND account_id = ?2",
                params![path, account_id, State::Done.as_str()],
            )
            .await?;
        Ok(())
    }

    /// Files whose path starts with `prefix`, with their confirmed accounts.
    pub async fn list(&self, prefix: &str) -> Result<Vec<IndexedFile>, Error> {
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
            match files.last_mut() {
                Some(last) if last.path == path => last.accounts.push(account),
                _ => files.push(IndexedFile {
                    path,
                    size: u64::try_from(size).unwrap_or(0),
                    accounts: vec![account],
                }),
            }
        }
        Ok(files)
    }

    /// Every placement (pending and done) whose path starts with `prefix`.
    pub async fn placements(&self, prefix: &str) -> Result<Vec<Placement>, Error> {
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
            let size: i64 = row.get(1)?;
            let state: String = row.get(3)?;
            out.push(Placement {
                path: row.get(0)?,
                size: u64::try_from(size).unwrap_or(0),
                account_id: row.get(2)?,
                state: if state == "done" {
                    State::Done
                } else {
                    State::Pending
                },
            });
        }
        Ok(out)
    }

    /// Forgets the placement of `path` on `account_id`, and the file itself
    /// when no placement is left. Touches the index only, never the remote.
    pub async fn remove_placement(&self, path: &str, account_id: &str) -> Result<(), Error> {
        self.conn
            .execute(
                "DELETE FROM placements WHERE path = ?1 AND account_id = ?2",
                params![path, account_id],
            )
            .await?;
        self.conn
            .execute(
                "DELETE FROM items WHERE path = ?1
                 AND NOT EXISTS (SELECT 1 FROM placements WHERE placements.path = ?1)",
                params![path],
            )
            .await?;
        Ok(())
    }

    /// Uploads that were started and never confirmed.
    pub async fn pending(&self) -> Result<Vec<PendingPlacement>, Error> {
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
