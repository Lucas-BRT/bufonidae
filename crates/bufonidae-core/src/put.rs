//! `put`: copy local files and folders into a pool of accounts, one file at a
//! time, each file on the account the placement policy picks.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use bufonidae_rclone::rcd::Rcd;

use crate::account::Account;
use crate::config::Config;
use crate::index::{self, Index};
use crate::placement::{AccountId, AccountSpace, Placement, Wanted, plan_batch_copies};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid destination {0:?}: must be relative and free of '..'")]
    InvalidDestination(String),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("remote: {0}")]
    Remote(String),
    #[error(transparent)]
    Index(#[from] index::Error),
    #[error("account {0:?} was given twice; each copy must go to a different account")]
    DuplicateAccount(String),
}

/// What `put` needs from a storage account. Implemented over `rclone rcd` for
/// real use and by a deterministic fake in tests.
pub trait RemoteFs {
    /// Free bytes of the account, `None` when the backend does not say.
    fn free_space(&self, account: &Account) -> impl Future<Output = Result<Option<u64>, Error>>;
    /// Every file under `dir` as `(path relative to dir, size in bytes)`. A
    /// folder that does not exist yet is an empty listing; any other failure
    /// is an error, never an empty listing.
    fn list_files(
        &self,
        account: &Account,
        dir: &str,
    ) -> impl Future<Output = Result<HashMap<String, u64>, Error>>;
    /// Copies `remote_path` from the account to `dest_dir/dest_rel` on the
    /// local disk.
    fn download(
        &self,
        account: &Account,
        remote_path: &str,
        dest_dir: &Path,
        dest_rel: &str,
    ) -> impl Future<Output = Result<(), Error>>;
    /// Copies `parent/name` from the local disk to `remote_path` on the account.
    fn upload(
        &self,
        account: &Account,
        parent: &Path,
        name: &str,
        remote_path: &str,
    ) -> impl Future<Output = Result<(), Error>>;
}

impl RemoteFs for Rcd {
    async fn free_space(&self, account: &Account) -> Result<Option<u64>, Error> {
        let about = self
            .about(&format!("{}:", account.remote_name()))
            .await
            .map_err(|e| Error::Remote(e.to_string()))?;
        Ok(about.free)
    }

    async fn list_files(
        &self,
        account: &Account,
        dir: &str,
    ) -> Result<HashMap<String, u64>, Error> {
        match self
            .list_files_recursive(&format!("{}:", account.remote_name()), dir)
            .await
        {
            Ok(entries) => Ok(entries
                .into_iter()
                .map(|e| (e.path, u64::try_from(e.size).unwrap_or(0)))
                .collect()),
            // The folder does not exist yet on this account. Only rclone's own
            // "directory not found" answer counts: any other failure must stay
            // an error, or a reconcile would read it as "everything is gone".
            Err(e) if e.is_directory_not_found() => Ok(HashMap::new()),
            Err(e) => Err(Error::Remote(e.to_string())),
        }
    }

    async fn download(
        &self,
        account: &Account,
        remote_path: &str,
        dest_dir: &Path,
        dest_rel: &str,
    ) -> Result<(), Error> {
        self.copy_file(
            &format!("{}:", account.remote_name()),
            remote_path,
            &dest_dir.to_string_lossy(),
            dest_rel,
        )
        .await
        .map_err(|e| Error::Remote(e.to_string()))
    }

    async fn upload(
        &self,
        account: &Account,
        parent: &Path,
        name: &str,
        remote_path: &str,
    ) -> Result<(), Error> {
        self.copy_file(
            &parent.to_string_lossy(),
            name,
            &format!("{}:", account.remote_name()),
            remote_path,
        )
        .await
        .map_err(|e| Error::Remote(e.to_string()))
    }
}

/// A local file found while scanning, with the state it had then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFile {
    pub abs: PathBuf,
    /// Path inside the destination, `/`-separated.
    pub rel: String,
    pub size: u64,
    pub mtime: SystemTime,
}

#[derive(Debug, Default)]
pub struct Scan {
    pub files: Vec<LocalFile>,
    /// Symlinks and non-UTF-8 names, which are never followed or uploaded.
    pub skipped: Vec<PathBuf>,
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    |source| Error::Io {
        path: path.to_owned(),
        source,
    }
}

/// Collects the files under `sources`. A directory keeps its own name as the
/// first component of `rel`. Symlinks are skipped, not followed.
pub fn scan(sources: &[PathBuf]) -> Result<Scan, Error> {
    let mut out = Scan::default();
    for source in sources {
        let meta = fs::symlink_metadata(source).map_err(io_err(source))?;
        let abs = fs::canonicalize(source).map_err(io_err(source))?;
        let name = abs
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_owned)
            .ok_or_else(|| Error::Io {
                path: source.clone(),
                source: std::io::Error::other("path has no UTF-8 file name"),
            })?;
        if meta.is_symlink() {
            out.skipped.push(source.clone());
        } else if meta.is_dir() {
            walk(&abs, &name, &mut out)?;
        } else {
            push_file(&abs, name, &meta, &mut out)?;
        }
    }
    Ok(out)
}

fn walk(dir: &Path, rel: &str, out: &mut Scan) -> Result<(), Error> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(io_err(dir))?
        .collect::<Result<_, _>>()
        .map_err(io_err(dir))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            out.skipped.push(path);
            continue;
        };
        let meta = fs::symlink_metadata(&path).map_err(io_err(&path))?;
        let child = format!("{rel}/{name}");
        if meta.is_symlink() {
            out.skipped.push(path);
        } else if meta.is_dir() {
            walk(&path, &child, out)?;
        } else if meta.is_file() {
            push_file(&path, child, &meta, out)?;
        } else {
            out.skipped.push(path);
        }
    }
    Ok(())
}

fn push_file(path: &Path, rel: String, meta: &fs::Metadata, out: &mut Scan) -> Result<(), Error> {
    out.files.push(LocalFile {
        abs: path.to_owned(),
        rel,
        size: meta.len(),
        mtime: meta.modified().map_err(io_err(path))?,
    });
    Ok(())
}

fn normalize_dest(dest: &str) -> Result<String, Error> {
    let trimmed = dest.trim_matches('/');
    let bad = dest.starts_with('/')
        || trimmed
            .split('/')
            .any(|c| c == ".." || c == "." && trimmed != ".");
    if bad {
        return Err(Error::InvalidDestination(dest.to_owned()));
    }
    Ok(if trimmed == "." {
        String::new()
    } else {
        trimmed.to_owned()
    })
}

fn join(dest: &str, rel: &str) -> String {
    if dest.is_empty() {
        rel.to_owned()
    } else {
        format!("{dest}/{rel}")
    }
}

/// Outcome of a `put`. Nothing is silently dropped: every scanned file ends
/// up in exactly one of these lists.
#[derive(Debug, Default)]
pub struct Report {
    /// `(rel path, account label, bytes)`
    pub uploaded: Vec<(String, String, u64)>,
    /// Already present on some account at the destination.
    pub already_there: Vec<String>,
    /// No account had room.
    pub no_room: Vec<String>,
    /// `(rel path, reason)`
    pub failed: Vec<(String, String)>,
    pub skipped: Vec<PathBuf>,
}

impl Report {
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.no_room.is_empty() && self.failed.is_empty()
    }
}

fn changed_since_scan(file: &LocalFile) -> bool {
    fs::metadata(&file.abs)
        .and_then(|m| m.modified().map(|t| (m.len(), t)))
        .map_or(true, |(len, time)| len != file.size || time != file.mtime)
}

/// A step of a running [`put_observed`], for progress displays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    /// The batch is planned. A file with two copies counts twice.
    Planned { copies: usize, bytes: u64 },
    /// One copy was stored.
    Copied {
        rel: String,
        account: String,
        size: u64,
    },
    /// One copy failed.
    Failed { rel: String, reason: String },
}

/// Copies `sources` into `dest` on the given `accounts`.
pub async fn put<R: RemoteFs>(
    remote: &R,
    index: &Index,
    accounts: &[Account],
    policy: &impl Placement,
    config: &Config,
    sources: &[PathBuf],
    dest: &str,
) -> Result<Report, Error> {
    put_observed(
        remote,
        index,
        accounts,
        policy,
        config,
        sources,
        dest,
        &mut |_| {},
    )
    .await
}

/// [`put`], reporting each step to `observer` as it happens.
#[allow(clippy::too_many_arguments)]
pub async fn put_observed<R: RemoteFs>(
    remote: &R,
    index: &Index,
    accounts: &[Account],
    policy: &impl Placement,
    config: &Config,
    sources: &[PathBuf],
    dest: &str,
    observer: &mut (dyn FnMut(Progress) + Send),
) -> Result<Report, Error> {
    let dest = normalize_dest(dest)?;
    // The same account twice would let two "different" copies share one
    // account, defeating mirroring.
    let mut seen = HashSet::new();
    for account in accounts {
        if !seen.insert(account.id.as_str()) {
            return Err(Error::DuplicateAccount(account.label.clone()));
        }
    }
    let scan = scan(sources)?;
    let mut report = Report {
        skipped: scan.skipped,
        ..Report::default()
    };

    // Never create a duplicate: a file already on any account is left alone.
    let mut present = HashSet::new();
    let mut spaces = Vec::new();
    for (i, account) in accounts.iter().enumerate() {
        present.extend(remote.list_files(account, &dest).await?.into_keys());
        let free = remote.free_space(account).await?.ok_or_else(|| {
            Error::Remote(format!(
                "account {:?} does not report its free space",
                account.label
            ))
        })?;
        spaces.push(AccountSpace {
            id: AccountId(u32::try_from(i).map_err(|e| Error::Remote(e.to_string()))?),
            free_bytes: free,
        });
    }

    let (todo, there): (Vec<_>, Vec<_>) = scan
        .files
        .into_iter()
        .partition(|f| !present.contains(&f.rel));
    report.already_there = there.into_iter().map(|f| f.rel).collect();

    let wanted: Vec<Wanted> = todo
        .iter()
        .map(|f| Wanted {
            size: f.size,
            copies: config.copies_for(&join(&dest, &f.rel)),
        })
        .collect();
    let plan = plan_batch_copies(policy, &spaces, &wanted);
    observer(Progress::Planned {
        copies: plan.iter().flatten().map(Vec::len).sum(),
        bytes: todo
            .iter()
            .zip(&plan)
            .map(|(f, p)| f.size * p.as_ref().map_or(0, |ids| ids.len() as u64))
            .sum(),
    });

    for (file, chosen) in todo.iter().zip(plan) {
        // `None`: not enough accounts or space for every wanted copy.
        let Some(targets) = chosen else {
            report.no_room.push(file.rel.clone());
            continue;
        };
        if changed_since_scan(file) {
            report
                .failed
                .push((file.rel.clone(), "changed while the put was running".into()));
            continue;
        }
        let (Some(parent), Some(name)) = (
            file.abs.parent(),
            file.abs.file_name().and_then(|n| n.to_str()),
        ) else {
            report
                .failed
                .push((file.rel.clone(), "unusable path".into()));
            continue;
        };
        let remote_path = join(&dest, &file.rel);
        for AccountId(slot) in targets {
            let account = &accounts[slot as usize];
            // Write-ahead: a crash after this line leaves a `pending` trace.
            index
                .mark_pending(&remote_path, file.size, &account.id)
                .await?;
            match remote.upload(account, parent, name, &remote_path).await {
                Ok(()) => {
                    index.mark_done(&remote_path, &account.id).await?;
                    report
                        .uploaded
                        .push((file.rel.clone(), account.label.clone(), file.size));
                    observer(Progress::Copied {
                        rel: file.rel.clone(),
                        account: account.label.clone(),
                        size: file.size,
                    });
                }
                // Left `pending` on purpose: a failed upload may have left a
                // partial file for `reconcile` to find. The other copies are
                // still attempted; `repair` completes the set later.
                Err(e) => {
                    let reason = format!("{}: {e}", account.label);
                    observer(Progress::Failed {
                        rel: file.rel.clone(),
                        reason: reason.clone(),
                    });
                    report.failed.push((file.rel.clone(), reason));
                }
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::placement::MostFreeSpace;
    use std::cell::RefCell;
    use std::collections::HashMap;

    struct Fake {
        fail_uploads: bool,
        free: HashMap<String, u64>,
        existing: HashMap<String, HashSet<String>>,
        uploads: RefCell<Vec<(String, String)>>,
    }

    impl RemoteFs for Fake {
        async fn free_space(&self, a: &Account) -> Result<Option<u64>, Error> {
            Ok(self.free.get(&a.label).copied())
        }
        async fn list_files(&self, a: &Account, _: &str) -> Result<HashMap<String, u64>, Error> {
            Ok(self
                .existing
                .get(&a.label)
                .into_iter()
                .flatten()
                .map(|p| (p.clone(), 0))
                .collect())
        }
        async fn download(&self, _: &Account, _: &str, _: &Path, _: &str) -> Result<(), Error> {
            Err(Error::Remote("not used by put".into()))
        }
        async fn upload(&self, a: &Account, _: &Path, _: &str, to: &str) -> Result<(), Error> {
            if self.fail_uploads {
                return Err(Error::Remote("boom".into()));
            }
            self.uploads
                .borrow_mut()
                .push((a.label.clone(), to.to_owned()));
            Ok(())
        }
    }

    fn account(label: &str) -> Account {
        Account {
            id: label.to_owned(),
            label: label.to_owned(),
            email: format!("{label}@example.com"),
        }
    }

    fn fake(free: &[(&str, u64)]) -> Fake {
        Fake {
            fail_uploads: false,
            free: free.iter().map(|(l, f)| ((*l).to_owned(), *f)).collect(),
            existing: HashMap::new(),
            uploads: RefCell::default(),
        }
    }

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("photos");
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("a.bin"), vec![0; 60]).unwrap();
        fs::write(root.join("sub/b.bin"), vec![0; 50]).unwrap();
        std::os::unix::fs::symlink(root.join("a.bin"), root.join("link")).unwrap();
        dir
    }

    #[test]
    fn scan_keeps_folder_name_and_skips_symlinks() {
        let dir = tree();
        let s = scan(&[dir.path().join("photos")]).unwrap();
        let rels: Vec<_> = s.files.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(rels, ["photos/a.bin", "photos/sub/b.bin"]);
        assert_eq!(s.skipped.len(), 1);
    }

    #[test]
    fn destination_is_validated() {
        for bad in ["/abs", "a/../b", ".."] {
            assert!(normalize_dest(bad).is_err(), "{bad}");
        }
        assert_eq!(normalize_dest("Pics/2026/").unwrap(), "Pics/2026");
        assert_eq!(normalize_dest(".").unwrap(), "");
    }

    #[tokio::test]
    async fn spreads_files_across_accounts() {
        let dir = tree();
        let remote = fake(&[("one", 100), ("two", 100)]);
        let index = Index::open_memory().await.unwrap();
        let report = put(
            &remote,
            &index,
            &[account("one"), account("two")],
            &MostFreeSpace::default(),
            &Config::default(),
            &[dir.path().join("photos")],
            "Pics",
        )
        .await
        .unwrap();
        assert!(report.is_complete());
        assert_eq!(report.uploaded.len(), 2);
        let indexed = index.list("Pics/").await.unwrap();
        assert_eq!(indexed.len(), 2);
        assert!(index.pending().await.unwrap().is_empty());
        let uploads = remote.uploads.borrow();
        assert_ne!(uploads[0].0, uploads[1].0, "one file per account");
        assert!(uploads.iter().any(|(_, to)| to == "Pics/photos/sub/b.bin"));
    }

    #[tokio::test]
    async fn skips_files_present_on_any_account_and_reports_no_room() {
        let dir = tree();
        let mut remote = fake(&[("one", 55), ("two", 10)]);
        remote
            .existing
            .insert("two".into(), HashSet::from(["photos/sub/b.bin".to_owned()]));
        let index = Index::open_memory().await.unwrap();
        let report = put(
            &remote,
            &index,
            &[account("one"), account("two")],
            &MostFreeSpace::default(),
            &Config::default(),
            &[dir.path().join("photos")],
            "Pics",
        )
        .await
        .unwrap();
        assert_eq!(report.already_there, ["photos/sub/b.bin"]);
        assert_eq!(report.no_room, ["photos/a.bin"]);
        assert!(!report.is_complete());
        assert!(remote.uploads.borrow().is_empty());
    }

    #[tokio::test]
    async fn unknown_free_space_is_an_error() {
        let dir = tree();
        let remote = fake(&[]);
        let index = Index::open_memory().await.unwrap();
        let result = put(
            &remote,
            &index,
            &[account("one")],
            &MostFreeSpace::default(),
            &Config::default(),
            &[dir.path().join("photos")],
            "Pics",
        )
        .await;
        assert!(matches!(result, Err(Error::Remote(_))));
    }

    #[tokio::test]
    async fn failed_upload_stays_pending_in_the_index() {
        let dir = tree();
        let mut remote = fake(&[("one", 1000)]);
        remote.fail_uploads = true;
        let index = Index::open_memory().await.unwrap();
        let report = put(
            &remote,
            &index,
            &[account("one")],
            &MostFreeSpace::default(),
            &Config::default(),
            &[dir.path().join("photos")],
            "Pics",
        )
        .await
        .unwrap();
        assert_eq!(report.failed.len(), 2);
        assert!(index.list("").await.unwrap().is_empty());
        assert_eq!(index.pending().await.unwrap().len(), 2);
    }

    fn mirror(copies: u8) -> Config {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Config::load(dir.path().join("config.toml")).unwrap();
        c.set(None, copies).unwrap();
        c
    }

    #[tokio::test]
    async fn mirror_puts_each_file_on_distinct_accounts() {
        let dir = tree();
        let remote = fake(&[("one", 1000), ("two", 1000), ("three", 1000)]);
        let index = Index::open_memory().await.unwrap();
        let report = put(
            &remote,
            &index,
            &[account("one"), account("two"), account("three")],
            &MostFreeSpace::default(),
            &mirror(2),
            &[dir.path().join("photos")],
            "Pics",
        )
        .await
        .unwrap();
        assert!(report.is_complete());
        assert_eq!(report.uploaded.len(), 4, "2 files x 2 copies");
        for f in index.list("Pics/").await.unwrap() {
            assert_eq!(f.accounts.len(), 2, "{}", f.path);
        }
    }

    #[tokio::test]
    async fn too_few_accounts_for_the_copies_places_nothing() {
        let dir = tree();
        let remote = fake(&[("one", 1000)]);
        let index = Index::open_memory().await.unwrap();
        let report = put(
            &remote,
            &index,
            &[account("one")],
            &MostFreeSpace::default(),
            &mirror(2),
            &[dir.path().join("photos")],
            "Pics",
        )
        .await
        .unwrap();
        assert_eq!(report.no_room.len(), 2);
        assert!(remote.uploads.borrow().is_empty());
        assert!(index.placements("").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn folder_override_beats_the_default() {
        let dir = tree();
        let remote = fake(&[("one", 1000), ("two", 1000)]);
        let index = Index::open_memory().await.unwrap();
        let cfg_dir = tempfile::tempdir().unwrap();
        let mut config = Config::load(cfg_dir.path().join("config.toml")).unwrap();
        config.set(Some("Pics/photos/sub"), 2).unwrap();
        put(
            &remote,
            &index,
            &[account("one"), account("two")],
            &MostFreeSpace::default(),
            &config,
            &[dir.path().join("photos")],
            "Pics",
        )
        .await
        .unwrap();
        let files = index.list("Pics/").await.unwrap();
        let by_path = |p: &str| files.iter().find(|f| f.path == p).unwrap().accounts.len();
        assert_eq!(by_path("Pics/photos/a.bin"), 1);
        assert_eq!(by_path("Pics/photos/sub/b.bin"), 2);
    }

    #[tokio::test]
    async fn observer_sees_the_plan_and_every_copy() {
        let dir = tree();
        let remote = fake(&[("one", 1000), ("two", 1000)]);
        let index = Index::open_memory().await.unwrap();
        let mut events = Vec::new();
        let report = put_observed(
            &remote,
            &index,
            &[account("one"), account("two")],
            &MostFreeSpace::default(),
            &mirror(2),
            &[dir.path().join("photos")],
            "Pics",
            &mut |p| events.push(p),
        )
        .await
        .unwrap();
        assert!(report.is_complete());
        // photos/a.bin = 60 bytes, photos/sub/b.bin = 50 bytes, two copies each.
        assert_eq!(
            events[0],
            Progress::Planned {
                copies: 4,
                bytes: 220
            }
        );
        let copied = events
            .iter()
            .filter(|e| matches!(e, Progress::Copied { .. }))
            .count();
        assert_eq!(copied, 4);
        assert_eq!(events.len(), 5);
    }

    #[tokio::test]
    async fn the_same_account_listed_twice_is_refused() {
        let dir = tree();
        let remote = fake(&[("one", 1000)]);
        let index = Index::open_memory().await.unwrap();
        let result = put(
            &remote,
            &index,
            &[account("one"), account("one")],
            &MostFreeSpace::default(),
            &mirror(2),
            &[dir.path().join("photos")],
            "Pics",
        )
        .await;
        assert!(matches!(result, Err(Error::DuplicateAccount(_))));
        assert!(remote.uploads.borrow().is_empty());
    }
}
