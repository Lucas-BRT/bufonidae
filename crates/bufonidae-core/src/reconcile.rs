//! `reconcile`: compare the index with what the accounts really hold.
//!
//! The remote is the source of truth, the index a cache. Only the index is
//! ever changed, and only with `apply`; nothing is deleted or rewritten on a
//! remote. What cannot be settled from listings alone (a size that differs)
//! is reported and left for the user.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::account::Account;
use crate::get::is_under;
use crate::index::{self, Index, State};
use crate::put::{Listing, RemoteFs};

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
    /// Names that exist twice in one account. Which object is ours is
    /// unknowable, so they are left alone. `(path, account label)`.
    pub duplicates: Vec<Where>,
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
            + self.duplicates.len()
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
///
/// Anything ambiguous is quarantined: reported, never imported, confirmed or
/// forgotten. That covers a name that appears twice in one account and a path
/// whose size differs between accounts.
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
    let mut listings: HashMap<&str, Listing> = HashMap::new();
    for account in accounts {
        match remote.list_files(account, prefix).await {
            Ok(listing) => {
                let files = listing
                    .files
                    .into_iter()
                    .map(|(rel, size)| (join(prefix, &rel), size))
                    .collect();
                let duplicates = listing
                    .duplicates
                    .into_iter()
                    .map(|rel| join(prefix, &rel))
                    .collect();
                listings.insert(account.id.as_str(), Listing { files, duplicates });
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
        if listing.duplicates.contains(&row.path) {
            // Two objects share the name: which one is ours is unknowable.
            report.duplicates.push(at);
            continue;
        }
        match (row.state, listing.files.get(&row.path).copied()) {
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

    // On the remote, unknown to the index. Gather every candidate for a path
    // first, so that disagreeing accounts are seen together instead of the
    // second import quietly overwriting the first.
    let mut candidates: BTreeMap<&str, Vec<(&Account, u64)>> = BTreeMap::new();
    for account in accounts {
        let Some(listing) = listings.get(account.id.as_str()) else {
            continue;
        };
        for (path, &size) in &listing.files {
            if !seen.contains(&(path.as_str(), account.id.as_str())) {
                candidates.entry(path).or_default().push((account, size));
            }
        }
        for path in &listing.duplicates {
            if !seen.contains(&(path.as_str(), account.id.as_str())) {
                report
                    .duplicates
                    .push((path.clone(), account.label.clone()));
            }
        }
    }
    for (path, found) in candidates {
        let agreed = match indexed_size.get(path) {
            Some(&known) => known,
            None => found[0].1,
        };
        let all_agree = found.iter().all(|&(_, size)| size == agreed);
        for (account, size) in found {
            if all_agree || size == agreed && indexed_size.contains_key(path) {
                if apply {
                    index.mark_pending(path, size, &account.id).await?;
                    index.mark_done(path, &account.id).await?;
                }
                report
                    .imported
                    .push((path.to_owned(), account.label.clone(), size));
            } else {
                report
                    .conflicts
                    .push((path.to_owned(), account.label.clone(), agreed, size));
            }
        }
    }
    report.duplicates.sort();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::put::Error as RemoteError;
    use std::path::Path;

    /// `None` makes the account's listing fail.
    struct Fake(HashMap<String, Option<Vec<(String, u64)>>>);

    impl RemoteFs for Fake {
        async fn free_space(&self, _: &Account) -> Result<Option<u64>, RemoteError> {
            Ok(None)
        }
        async fn list_files(&self, a: &Account, dir: &str) -> Result<Listing, RemoteError> {
            let all = self.0[&a.id]
                .clone()
                .ok_or_else(|| RemoteError::Remote("network down".into()))?;
            // Like rclone: paths relative to `dir`.
            Ok(Listing::from_entries(
                all.into_iter()
                    .filter(|(p, _)| is_under(p, dir))
                    .map(|(p, s)| {
                        let rel = if dir.is_empty() {
                            p
                        } else {
                            p[dir.len() + 1..].to_owned()
                        };
                        (rel, s)
                    }),
            ))
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

    fn listing(files: &[(&str, u64)]) -> Option<Vec<(String, u64)>> {
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

    #[tokio::test]
    async fn accounts_that_disagree_on_a_new_path_are_quarantined_not_imported() {
        let index = Index::open_memory().await.unwrap();
        let remote = Fake(HashMap::from([
            ("a".to_owned(), listing(&[("x.bin", 5), ("ok.bin", 1)])),
            ("b".to_owned(), listing(&[("x.bin", 9), ("ok.bin", 1)])),
        ]));
        let r = reconcile(&remote, &index, &[account("a"), account("b")], "", true)
            .await
            .unwrap();
        // The agreeing file is imported on both; the disagreeing one on neither.
        assert_eq!(r.imported.len(), 2);
        assert!(r.imported.iter().all(|(p, _, _)| p == "ok.bin"));
        assert_eq!(r.conflicts.len(), 2, "both copies of x.bin are reported");
        let paths: Vec<_> = index
            .placements("")
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.path)
            .collect();
        assert!(!paths.contains(&"x.bin".to_owned()), "nothing was guessed");
        assert!(r.unresolved() > 0);
    }

    #[tokio::test]
    async fn a_duplicated_name_is_reported_and_never_touched() {
        let index = Index::open_memory().await.unwrap();
        seed(&index, &[("twin.bin", 5, "a", State::Done)]).await;
        let remote = Fake(HashMap::from([(
            "a".to_owned(),
            Some(vec![
                ("twin.bin".to_owned(), 5),
                ("twin.bin".to_owned(), 8),
                ("new.bin".to_owned(), 3),
            ]),
        )]));
        let r = reconcile(&remote, &index, &[account("a")], "", true)
            .await
            .unwrap();
        assert_eq!(r.duplicates, [("twin.bin".to_owned(), "a".to_owned())]);
        assert!(r.dropped.is_empty(), "an ambiguous name is not 'missing'");
        assert_eq!(r.imported.len(), 1, "only new.bin");
        assert_eq!(index.placements("twin").await.unwrap().len(), 1);
        assert!(r.unresolved() > 0);
    }

    #[tokio::test]
    async fn an_unknown_duplicated_name_is_reported_not_imported() {
        let index = Index::open_memory().await.unwrap();
        let remote = Fake(HashMap::from([(
            "a".to_owned(),
            Some(vec![("t.bin".to_owned(), 1), ("t.bin".to_owned(), 2)]),
        )]));
        let r = reconcile(&remote, &index, &[account("a")], "", true)
            .await
            .unwrap();
        assert_eq!(r.duplicates.len(), 1);
        assert!(r.imported.is_empty());
        assert!(index.placements("").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn losing_the_last_copy_is_remembered_so_repair_can_say_lost() {
        let index = Index::open_memory().await.unwrap();
        seed(&index, &[("gone.bin", 5, "a", State::Done)]).await;
        let remote = Fake(HashMap::from([("a".to_owned(), listing(&[]))]));
        let r = reconcile(&remote, &index, &[account("a")], "", true)
            .await
            .unwrap();
        assert_eq!(r.dropped.len(), 1);
        assert_eq!(index.lost("").await.unwrap().len(), 1);
    }
}
