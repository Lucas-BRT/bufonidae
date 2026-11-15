//! `reconcile`: compare the index with what the accounts really hold.
//!
//! The remote is the source of truth, the index a cache. Only the index is
//! ever changed, and only with `apply`; nothing is deleted or rewritten on a
//! remote. What cannot be settled from listings alone (a size that differs)
//! is reported and left for the user.

use std::collections::{HashMap, HashSet};

use crate::account::Account;
use crate::get::is_under;
use crate::index::{self, Index, State};
use crate::put::RemoteFs;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Index(#[from] index::Error),
}

/// A file on an account, as `(path, account label)`.
pub type Where = (String, String);

#[derive(Debug, Default)]
pub struct Report {
    /// Index and remote agree.
    pub consistent: usize,
    /// Pending in the index, complete on the remote: now `done`.
    pub confirmed: Vec<Where>,
    /// In the index but not on the remote (lost, or an upload that never
    /// happened): forgotten.
    pub dropped: Vec<Where>,
    /// On the remote but not in the index: adopted, as `(path, label, bytes)`.
    pub imported: Vec<(String, String, u64)>,
    /// Confirmed in the index, but the remote size differs.
    /// `(path, label, indexed bytes, remote bytes)`. Reported only.
    pub size_mismatch: Vec<(String, String, u64, u64)>,
    /// Pending in the index and present on the remote with another size: a
    /// partial upload. Reported only; the file is left in place.
    pub partial: Vec<(String, String, u64, u64)>,
    /// On the remote with a size that contradicts what the index holds for
    /// the same path on another account. Reported only.
    pub conflicts: Vec<(String, String, u64, u64)>,
    /// Accounts whose listing failed, with the error. Their entries are left
    /// untouched.
    pub unreachable: Vec<(String, String)>,
    /// Index entries on accounts that are not connected.
    pub unverifiable: usize,
    /// Whether the fixes above were written to the index.
    pub applied: bool,
}

impl Report {
    /// Differences the index can fix by itself.
    #[must_use]
    pub fn fixable(&self) -> usize {
        self.confirmed.len() + self.dropped.len() + self.imported.len()
    }

    /// Problems that need a human decision or a retry.
    #[must_use]
    pub fn unresolved(&self) -> usize {
        self.size_mismatch.len()
            + self.partial.len()
            + self.conflicts.len()
            + self.unreachable.len()
    }
}

fn join(prefix: &str, rel: &str) -> String {
    if prefix.is_empty() {
        rel.to_owned()
    } else {
        format!("{prefix}/{rel}")
    }
}

/// Reconciles the files under `prefix` (all of them when empty) on `accounts`.
pub async fn reconcile<R: RemoteFs>(
    remote: &R,
    index: &Index,
    accounts: &[Account],
    prefix: &str,
    apply: bool,
) -> Result<Report, Error> {
    let prefix = prefix.trim_matches('/');
    let mut report = Report {
        applied: apply,
        ..Report::default()
    };

    // A failed listing must never read as an empty one, or every entry of
    // that account would look lost.
    let mut listings: HashMap<&str, HashMap<String, u64>> = HashMap::new();
    for account in accounts {
        match remote.list_files(account, prefix).await {
            Ok(files) => {
                let full = files
                    .into_iter()
                    .map(|(rel, size)| (join(prefix, &rel), size));
                listings.insert(account.id.as_str(), full.collect());
            }
            Err(e) => report
                .unreachable
                .push((account.label.clone(), e.to_string())),
        }
    }
    let label = |id: &str| {
        accounts
            .iter()
            .find(|a| a.id == id)
            .map_or_else(|| id.to_owned(), |a| a.label.clone())
    };

    let rows: Vec<_> = index
        .placements(prefix)
        .await?
        .into_iter()
        .filter(|p| is_under(&p.path, prefix))
        .collect();
    let indexed_size: HashMap<&str, u64> = rows.iter().map(|p| (p.path.as_str(), p.size)).collect();

    let mut seen: HashSet<(&str, &str)> = HashSet::new();
    for row in &rows {
        let Some(listing) = listings.get(row.account_id.as_str()) else {
            // Not connected, or its listing failed (already reported).
            if !accounts.iter().any(|a| a.id == row.account_id) {
                report.unverifiable += 1;
            }
            continue;
        };
        seen.insert((row.path.as_str(), row.account_id.as_str()));
        let at = (row.path.clone(), label(&row.account_id));
        match (row.state, listing.get(&row.path).copied()) {
            (State::Done, Some(size)) if size == row.size => report.consistent += 1,
            (State::Done, Some(size)) => {
                report.size_mismatch.push((at.0, at.1, row.size, size));
            }
            (State::Pending, Some(size)) if size == row.size => {
                if apply {
                    index.mark_done(&row.path, &row.account_id).await?;
                }
                report.confirmed.push(at);
            }
            (State::Pending, Some(size)) => {
                report.partial.push((at.0, at.1, row.size, size));
            }
            (_, None) => {
                if apply {
                    index.remove_placement(&row.path, &row.account_id).await?;
                }
                report.dropped.push(at);
            }
        }
    }

    // On the remote, unknown to the index.
    for account in accounts {
        let Some(listing) = listings.get(account.id.as_str()) else {
            continue;
        };
        let mut paths: Vec<_> = listing.iter().collect();
        paths.sort();
        for (path, &size) in paths {
            if seen.contains(&(path.as_str(), account.id.as_str())) {
                continue;
            }
            if let Some(&known) = indexed_size.get(path.as_str())
                && known != size
            {
                report
                    .conflicts
                    .push((path.clone(), account.label.clone(), known, size));
                continue;
            }
            if apply {
                index.mark_pending(path, size, &account.id).await?;
                index.mark_done(path, &account.id).await?;
            }
            report
                .imported
                .push((path.clone(), account.label.clone(), size));
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::put::Error as RemoteError;
    use std::path::Path;

    /// `None` makes the account's listing fail.
    struct Fake(HashMap<String, Option<HashMap<String, u64>>>);

    impl RemoteFs for Fake {
        async fn free_space(&self, _: &Account) -> Result<Option<u64>, RemoteError> {
            Ok(None)
        }
        async fn list_files(
            &self,
            a: &Account,
            dir: &str,
        ) -> Result<HashMap<String, u64>, RemoteError> {
            let all = self.0[&a.id]
                .clone()
                .ok_or_else(|| RemoteError::Remote("network down".into()))?;
            // Like rclone: paths relative to `dir`.
            Ok(all
                .into_iter()
                .filter(|(p, _)| is_under(p, dir))
                .map(|(p, s)| {
                    let rel = if dir.is_empty() {
                        p
                    } else {
                        p[dir.len() + 1..].to_owned()
                    };
                    (rel, s)
                })
                .collect())
        }
        async fn download(
            &self,
            _: &Account,
            _: &str,
            _: &Path,
            _: &str,
        ) -> Result<(), RemoteError> {
            Err(RemoteError::Remote("unused".into()))
        }
        async fn upload(&self, _: &Account, _: &Path, _: &str, _: &str) -> Result<(), RemoteError> {
            Err(RemoteError::Remote("unused".into()))
        }
    }

    fn account(id: &str) -> Account {
        Account {
            id: id.into(),
            label: id.into(),
            email: format!("{id}@example.com"),
        }
    }

    fn listing(files: &[(&str, u64)]) -> Option<HashMap<String, u64>> {
        Some(files.iter().map(|(p, s)| ((*p).to_owned(), *s)).collect())
    }

    async fn seed(index: &Index, rows: &[(&str, u64, &str, State)]) {
        for (path, size, acct, state) in rows {
            index.mark_pending(path, *size, acct).await.unwrap();
            if *state == State::Done {
                index.mark_done(path, acct).await.unwrap();
            }
        }
    }

    #[tokio::test]
    async fn classifies_every_difference_and_apply_fixes_only_the_safe_ones() {
        let index = Index::open_memory().await.unwrap();
        seed(
            &index,
            &[
                ("ok.bin", 10, "a", State::Done),
                ("lost.bin", 10, "a", State::Done),
                ("resized.bin", 10, "a", State::Done),
                ("finished.bin", 10, "a", State::Pending),
                ("halfway.bin", 10, "a", State::Pending),
                ("never.bin", 10, "a", State::Pending),
            ],
        )
        .await;
        let remote = Fake(HashMap::from([(
            "a".to_owned(),
            listing(&[
                ("ok.bin", 10),
                ("resized.bin", 99),
                ("finished.bin", 10),
                ("halfway.bin", 4),
                ("new.bin", 7),
            ]),
        )]));
        let accounts = [account("a")];

        // Dry run: reports, changes nothing.
        let dry = reconcile(&remote, &index, &accounts, "", false)
            .await
            .unwrap();
        assert_eq!(dry.consistent, 1);
        assert_eq!(dry.confirmed.len(), 1);
        assert_eq!(dry.dropped.len(), 2, "lost + never");
        assert_eq!(dry.imported.len(), 1);
        assert_eq!(dry.size_mismatch.len(), 1);
        assert_eq!(dry.partial.len(), 1);
        assert_eq!(index.placements("").await.unwrap().len(), 6);

        let applied = reconcile(&remote, &index, &accounts, "", true)
            .await
            .unwrap();
        assert_eq!(applied.fixable(), 4);

        // Fixed: only the two unresolved ones remain, and a rerun is stable.
        let again = reconcile(&remote, &index, &accounts, "", true)
            .await
            .unwrap();
        assert_eq!(again.fixable(), 0);
        assert_eq!(again.unresolved(), 2, "resized + halfway");
        let paths: Vec<_> = index
            .placements("")
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.path)
            .collect();
        assert!(!paths.contains(&"lost.bin".to_owned()));
        assert!(paths.contains(&"new.bin".to_owned()));
    }

    #[tokio::test]
    async fn unreachable_account_keeps_its_entries() {
        let index = Index::open_memory().await.unwrap();
        seed(&index, &[("a.bin", 5, "down", State::Done)]).await;
        let remote = Fake(HashMap::from([("down".to_owned(), None)]));

        let r = reconcile(&remote, &index, &[account("down")], "", true)
            .await
            .unwrap();
        assert_eq!(r.unreachable.len(), 1);
        assert!(r.dropped.is_empty());
        assert_eq!(index.placements("").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn disconnected_accounts_are_counted_not_touched() {
        let index = Index::open_memory().await.unwrap();
        seed(&index, &[("a.bin", 5, "ghost", State::Done)]).await;
        let remote = Fake(HashMap::from([("a".to_owned(), listing(&[]))]));

        let r = reconcile(&remote, &index, &[account("a")], "", true)
            .await
            .unwrap();
        assert_eq!(r.unverifiable, 1);
        assert_eq!(index.placements("").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn prefix_scopes_both_sides_and_conflicts_are_not_imported() {
        let index = Index::open_memory().await.unwrap();
        seed(
            &index,
            &[
                ("Pics/a.bin", 5, "a", State::Done),
                ("Docs/gone.bin", 5, "a", State::Done),
            ],
        )
        .await;
        let remote = Fake(HashMap::from([
            (
                "a".to_owned(),
                listing(&[("Pics/a.bin", 5), ("Docs/x.bin", 1)]),
            ),
            ("b".to_owned(), listing(&[("Pics/a.bin", 6)])),
        ]));
        let accounts = [account("a"), account("b")];

        let r = reconcile(&remote, &index, &accounts, "Pics", true)
            .await
            .unwrap();
        assert_eq!(r.consistent, 1);
        assert_eq!(r.conflicts.len(), 1, "b has Pics/a.bin with another size");
        assert!(r.imported.is_empty());
        assert!(r.dropped.is_empty(), "Docs is out of scope");
        assert_eq!(index.placements("Docs").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn imports_what_the_index_never_saw() {
        let index = Index::open_memory().await.unwrap();
        let remote = Fake(HashMap::from([
            ("a".to_owned(), listing(&[("x/1.bin", 3)])),
            ("b".to_owned(), listing(&[("x/1.bin", 3), ("x/2.bin", 4)])),
        ]));
        let r = reconcile(&remote, &index, &[account("a"), account("b")], "", true)
            .await
            .unwrap();
        assert_eq!(r.imported.len(), 3);
        let files = index.list("x").await.unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].accounts, ["a", "b"], "same file, two replicas");
    }
}
