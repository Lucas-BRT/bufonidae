//! `get`: bring a file, or a whole folder, back from the pool.

use std::path::{Component, Path, PathBuf};

use crate::account::Account;
use crate::index::{self, Index, IndexedFile};
use crate::put::{Error as RemoteError, RemoteFs};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("nothing stored under {0:?}")]
    NotFound(String),
    #[error(transparent)]
    Index(#[from] index::Error),
}

/// Outcome of a `get`. Every matching file lands in exactly one list.
#[derive(Debug, Default)]
pub struct Report {
    /// `(path, account label, bytes)`
    pub downloaded: Vec<(String, String, u64)>,
    /// A local file is already there: never overwritten.
    pub already_there: Vec<PathBuf>,
    /// `(path, reason)`
    pub failed: Vec<(String, String)>,
}

impl Report {
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.failed.is_empty()
    }
}

/// `true` when `path` is `prefix` itself or lives under it.
pub(crate) fn is_under(path: &str, prefix: &str) -> bool {
    prefix.is_empty()
        || path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The index is local state, so its paths are not trusted: a relative path
/// with a `..`, a root or an empty part must never reach the filesystem.
fn is_safe_relative(rel: &str) -> bool {
    !rel.is_empty()
        && Path::new(rel)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

/// Downloads `path` (a file or a folder) into `out`. A file lands directly in
/// `out`; a folder keeps its own name. Existing local files are left alone.
pub async fn get<R: RemoteFs>(
    remote: &R,
    index: &Index,
    accounts: &[Account],
    path: &str,
    out: &Path,
) -> Result<Report, Error> {
    get_observed(remote, index, accounts, path, out, &mut |_| {}).await
}

/// A step of a running [`get_observed`]. Every matching file produces exactly
/// one of `Downloaded`, `Skipped` or `Failed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    /// What will be attempted: how many files and their total size.
    Planned {
        files: usize,
        bytes: u64,
    },
    Downloaded {
        path: String,
        account: String,
        size: u64,
    },
    /// A local file was already there and is left alone.
    Skipped {
        path: String,
    },
    Failed {
        path: String,
        reason: String,
    },
}

fn fail(
    report: &mut Report,
    observer: &mut (dyn FnMut(Progress) + Send),
    path: String,
    reason: String,
) {
    observer(Progress::Failed {
        path: path.clone(),
        reason: reason.clone(),
    });
    report.failed.push((path, reason));
}

/// [`get`], reporting each step to `observer` as it happens.
pub async fn get_observed<R: RemoteFs>(
    remote: &R,
    index: &Index,
    accounts: &[Account],
    path: &str,
    out: &Path,
    observer: &mut (dyn FnMut(Progress) + Send),
) -> Result<Report, Error> {
    let path = path.trim_matches('/');
    let files: Vec<IndexedFile> = index
        .list(path)
        .await?
        .into_iter()
        .filter(|f| is_under(&f.path, path))
        .collect();
    if files.is_empty() {
        return Err(Error::NotFound(path.to_owned()));
    }

    observer(Progress::Planned {
        files: files.len(),
        bytes: files.iter().map(|f| f.size).sum(),
    });

    // Local names are relative to the parent of the requested path.
    let parent_len = path.rfind('/').map_or(0, |i| i + 1);
    let mut report = Report::default();

    for file in files {
        let rel = &file.path[parent_len.min(file.path.len())..];
        if !is_safe_relative(rel) {
            fail(
                &mut report,
                observer,
                file.path.clone(),
                "unsafe path in the index".into(),
            );
            continue;
        }
        let target = out.join(rel);
        if target.exists() {
            observer(Progress::Skipped {
                path: file.path.clone(),
            });
            report.already_there.push(target);
            continue;
        }

        let holders: Vec<&Account> = file
            .accounts
            .iter()
            .filter_map(|id| accounts.iter().find(|a| &a.id == id))
            .collect();
        if holders.is_empty() {
            fail(
                &mut report,
                observer,
                file.path.clone(),
                "none of the accounts holding it is connected".into(),
            );
            continue;
        }

        // Try each replica until one delivers a file of the expected size.
        let mut last_error = String::new();
        let mut done = None;
        for account in holders {
            match remote.download(account, &file.path, out, rel).await {
                Ok(()) => match std::fs::metadata(&target) {
                    Ok(m) if m.len() == file.size => {
                        done = Some(account.label.clone());
                        break;
                    }
                    Ok(m) => {
                        last_error = format!(
                            "{}: size mismatch (expected {}, got {})",
                            account.label,
                            file.size,
                            m.len()
                        );
                        let _ = std::fs::remove_file(&target);
                    }
                    Err(e) => last_error = format!("{}: {e}", account.label),
                },
                Err(RemoteError::Remote(e)) => last_error = format!("{}: {e}", account.label),
                Err(e) => last_error = format!("{}: {e}", account.label),
            }
        }
        match done {
            Some(label) => {
                observer(Progress::Downloaded {
                    path: file.path.clone(),
                    account: label.clone(),
                    size: file.size,
                });
                report.downloaded.push((file.path, label, file.size));
            }
            None => fail(&mut report, observer, file.path, last_error),
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::path::Path;

    /// Serves files of known sizes; accounts in `broken` always fail, and
    /// accounts in `short` deliver a truncated file.
    struct Fake {
        sizes: HashMap<String, u64>,
        broken: HashSet<String>,
        short: HashSet<String>,
    }

    impl RemoteFs for Fake {
        async fn free_space(&self, _: &Account) -> Result<Option<u64>, RemoteError> {
            Ok(None)
        }
        async fn list_files(
            &self,
            _: &Account,
            _: &str,
        ) -> Result<crate::put::Listing, RemoteError> {
            Ok(crate::put::Listing::default())
        }
        async fn upload(&self, _: &Account, _: &Path, _: &str, _: &str) -> Result<(), RemoteError> {
            Err(RemoteError::Remote("not used by get".into()))
        }
        async fn download(
            &self,
            account: &Account,
            remote_path: &str,
            dest_dir: &Path,
            dest_rel: &str,
        ) -> Result<(), RemoteError> {
            if self.broken.contains(&account.id) {
                return Err(RemoteError::Remote("quota exceeded".into()));
            }
            let mut len = self.sizes[remote_path];
            if self.short.contains(&account.id) {
                len -= 1;
            }
            let target = dest_dir.join(dest_rel);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, vec![7; usize::try_from(len).unwrap()]).unwrap();
            Ok(())
        }
    }

    fn account(id: &str) -> Account {
        Account {
            id: id.into(),
            label: id.into(),
            email: format!("{id}@example.com"),
        }
    }

    async fn indexed(entries: &[(&str, u64, &[&str])]) -> (Index, Fake) {
        let index = Index::open_memory().await.unwrap();
        let mut sizes = HashMap::new();
        for (path, size, accounts) in entries {
            sizes.insert((*path).to_owned(), *size);
            for a in *accounts {
                index.mark_pending(path, *size, a).await.unwrap();
                index.mark_done(path, a).await.unwrap();
            }
        }
        let fake = Fake {
            sizes,
            broken: HashSet::new(),
            short: HashSet::new(),
        };
        (index, fake)
    }

    #[tokio::test]
    async fn file_lands_directly_in_out_and_folder_keeps_its_name() {
        let (index, fake) = indexed(&[
            ("Pics/photos/a.bin", 5, &["one"]),
            ("Pics/photos/sub/b.bin", 3, &["two"]),
            ("Pics/photos2/c.bin", 9, &["one"]),
        ])
        .await;
        let accounts = [account("one"), account("two")];

        let out = tempfile::tempdir().unwrap();
        let r = get(&fake, &index, &accounts, "Pics/photos/a.bin", out.path())
            .await
            .unwrap();
        assert!(r.is_complete());
        assert!(out.path().join("a.bin").exists());

        let out = tempfile::tempdir().unwrap();
        let r = get(&fake, &index, &accounts, "Pics/photos", out.path())
            .await
            .unwrap();
        assert_eq!(r.downloaded.len(), 2, "photos2 is not under photos");
        assert!(out.path().join("photos/sub/b.bin").exists());
        assert!(!out.path().join("photos2").exists());
    }

    #[tokio::test]
    async fn never_overwrites_and_reports_missing() {
        let (index, fake) = indexed(&[("a.bin", 5, &["one"])]).await;
        let accounts = [account("one")];
        let out = tempfile::tempdir().unwrap();
        std::fs::write(out.path().join("a.bin"), b"mine").unwrap();

        let r = get(&fake, &index, &accounts, "a.bin", out.path())
            .await
            .unwrap();
        assert_eq!(r.already_there.len(), 1);
        assert_eq!(std::fs::read(out.path().join("a.bin")).unwrap(), b"mine");

        let missing = get(&fake, &index, &accounts, "nope", out.path()).await;
        assert!(matches!(missing, Err(Error::NotFound(_))));
    }

    #[tokio::test]
    async fn falls_back_to_another_replica() {
        let (index, mut fake) = indexed(&[("a.bin", 5, &["one", "two"])]).await;
        fake.broken.insert("one".into());
        let out = tempfile::tempdir().unwrap();
        let r = get(
            &fake,
            &index,
            &[account("one"), account("two")],
            "a.bin",
            out.path(),
        )
        .await
        .unwrap();
        assert_eq!(r.downloaded[0].1, "two");
    }

    #[tokio::test]
    async fn size_mismatch_is_a_failure_and_leaves_no_file() {
        let (index, mut fake) = indexed(&[("a.bin", 5, &["one"])]).await;
        fake.short.insert("one".into());
        let out = tempfile::tempdir().unwrap();
        let r = get(&fake, &index, &[account("one")], "a.bin", out.path())
            .await
            .unwrap();
        assert_eq!(r.failed.len(), 1);
        assert!(r.failed[0].1.contains("size mismatch"));
        assert!(!out.path().join("a.bin").exists());
    }

    #[tokio::test]
    async fn unsafe_index_paths_and_disconnected_accounts_fail_safely() {
        let (index, fake) = indexed(&[
            ("dir/../evil.bin", 1, &["one"]),
            ("ok/x.bin", 1, &["ghost"]),
        ])
        .await;
        let out = tempfile::tempdir().unwrap();
        let accounts = [account("one")];

        let r = get(&fake, &index, &accounts, "dir", out.path())
            .await
            .unwrap();
        assert!(r.failed[0].1.contains("unsafe"));
        assert!(!out.path().parent().unwrap().join("evil.bin").exists());

        let r = get(&fake, &index, &accounts, "ok/x.bin", out.path())
            .await
            .unwrap();
        assert!(r.failed[0].1.contains("connected"));
    }

    #[test]
    fn prefix_matching_respects_folder_boundaries() {
        assert!(is_under("a/b", "a"));
        assert!(is_under("a", "a"));
        assert!(!is_under("ab/c", "a"));
        assert!(is_under("anything", ""));
    }

    #[tokio::test]
    async fn observer_sees_the_plan_and_one_event_per_file() {
        let (index, fake) = indexed(&[
            ("d/a.bin", 5, &["one"]),
            ("d/b.bin", 3, &["one"]),
            ("d/c.bin", 2, &["ghost"]),
        ])
        .await;
        let out = tempfile::tempdir().unwrap();
        std::fs::create_dir(out.path().join("d")).unwrap();
        std::fs::write(out.path().join("d/b.bin"), b"mine").unwrap();

        let mut events = Vec::new();
        let report = get_observed(
            &fake,
            &index,
            &[account("one")],
            "d",
            out.path(),
            &mut |p| events.push(p),
        )
        .await
        .unwrap();

        assert_eq!(
            events[0],
            Progress::Planned {
                files: 3,
                bytes: 10
            }
        );
        assert_eq!(events.len(), 4, "plan + one event per file");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Progress::Downloaded { path, .. } if path == "d/a.bin"))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Progress::Skipped { path } if path == "d/b.bin"))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Progress::Failed { path, .. } if path == "d/c.bin"))
        );
        assert_eq!(report.downloaded.len(), 1);
    }
}
