//! Finding files that have fewer copies than wanted, and rebuilding the
//! missing copies (what a RAID rebuild does after a disk is lost).
//!
//! A rebuild downloads a file from a surviving account into a temporary
//! folder and uploads it to a new one, one file at a time.

use std::collections::HashMap;

use crate::account::Account;
use crate::config::Config;
use crate::get::is_under;
use crate::index::{self, Index};
use crate::placement::{AccountId, AccountSpace, Placement, choose_distinct};
use crate::put::{Error as RemoteError, RemoteFs};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Index(#[from] index::Error),
    #[error("temporary folder: {0}")]
    Temp(#[from] std::io::Error),
    #[error(transparent)]
    Remote(#[from] RemoteError),
}

/// A file with fewer live copies than the config asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gap {
    pub path: String,
    pub size: u64,
    /// Ids of connected accounts that hold a confirmed copy.
    pub holders: Vec<String>,
    pub wanted: u8,
    /// BLAKE3 recorded when it was stored, when known.
    pub blake3: Option<String>,
}

/// Files under `prefix` that lack copies on the connected accounts. A copy on
/// an account that is not connected does not count: it cannot be used now.
pub async fn find_gaps(
    index: &Index,
    accounts: &[Account],
    config: &Config,
    prefix: &str,
) -> Result<Vec<Gap>, Error> {
    let prefix = prefix.trim_matches('/');
    let mut gaps = Vec::new();
    for file in index.list(prefix).await? {
        if !is_under(&file.path, prefix) {
            continue;
        }
        let wanted = config.copies_for(&file.path);
        let holders: Vec<String> = file
            .accounts
            .into_iter()
            .filter(|id| accounts.iter().any(|a| &a.id == id))
            .collect();
        if holders.len() < usize::from(wanted) {
            gaps.push(Gap {
                path: file.path,
                size: file.size,
                holders,
                wanted,
                blake3: file.blake3,
            });
        }
    }
    // Files whose every copy is gone no longer appear in `list`, but they are
    // exactly what the user most needs to hear about.
    for lost in index.lost(prefix).await? {
        if is_under(&lost.path, prefix) {
            gaps.push(Gap {
                wanted: config.copies_for(&lost.path),
                path: lost.path,
                size: lost.size,
                holders: Vec::new(),
                blake3: None,
            });
        }
    }
    Ok(gaps)
}

#[derive(Debug, Default)]
pub struct Report {
    /// New copies made: `(path, account label)`.
    pub repaired: Vec<(String, String)>,
    /// No surviving copy on a connected account: cannot be rebuilt.
    pub lost: Vec<String>,
    /// Some copies could not be placed (no other account with room):
    /// `(path, copies still missing)`.
    pub no_room: Vec<(String, usize)>,
    /// `(path, reason)`
    pub failed: Vec<(String, String)>,
    /// Accounts that did not answer and were left out: `(label, reason)`.
    /// Copies they hold were not counted.
    pub unreachable: Vec<(String, String)>,
}

impl Report {
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.lost.is_empty() && self.no_room.is_empty() && self.failed.is_empty()
    }
}

const TEMP_NAME: &str = "blob";

/// Rebuilds the missing copies of every gap under `prefix`.
pub async fn repair<R: RemoteFs>(
    remote: &R,
    index: &Index,
    accounts: &[Account],
    policy: &impl Placement,
    config: &Config,
    prefix: &str,
) -> Result<Report, Error> {
    let mut report = Report::default();

    // Only an account that answers can be read from or written to. One that
    // does not is set aside and reported, instead of stopping the repair for
    // the accounts that work. Copies it holds are not counted while it is out.
    let mut usable: Vec<Account> = Vec::new();
    let mut spaces = Vec::new();
    for account in accounts {
        match remote.free_space(account).await {
            Ok(Some(free)) => {
                spaces.push(AccountSpace {
                    id: AccountId(u32::try_from(usable.len()).unwrap_or(u32::MAX)),
                    free_bytes: free,
                });
                usable.push(account.clone());
            }
            Ok(None) => report.unreachable.push((
                account.label.clone(),
                "it does not report its free space".to_owned(),
            )),
            Err(e) => report
                .unreachable
                .push((account.label.clone(), e.to_string())),
        }
    }
    let accounts = usable.as_slice();

    let mut gaps = find_gaps(index, accounts, config, prefix).await?;
    gaps.sort_by_key(|g| std::cmp::Reverse(g.size));
    let slot_of: HashMap<&str, AccountId> = accounts
        .iter()
        .zip(&spaces)
        .map(|(a, s)| (a.id.as_str(), s.id))
        .collect();

    for gap in gaps {
        if gap.holders.is_empty() {
            report.lost.push(gap.path);
            continue;
        }
        // Choose targets among the accounts that do not hold it yet.
        let mut taken: Vec<AccountId> = gap.holders.iter().map(|h| slot_of[h.as_str()]).collect();
        let missing = usize::from(gap.wanted) - gap.holders.len();
        let mut targets = Vec::new();
        for _ in 0..missing {
            let Some(id) = choose_distinct(policy, &spaces, &taken, gap.size) else {
                break;
            };
            taken.push(id);
            targets.push(id);
        }
        if targets.len() < missing {
            report
                .no_room
                .push((gap.path.clone(), missing - targets.len()));
        }
        if targets.is_empty() {
            continue;
        }

        // One download, then one upload per new copy.
        let tmp = tempfile::tempdir()?;
        let local = tmp.path().join(TEMP_NAME);
        let mut error = String::new();
        let mut downloaded = false;
        for holder in &gap.holders {
            let Some(account) = accounts.iter().find(|a| &a.id == holder) else {
                continue;
            };
            let fetched = remote
                .download(account, &gap.path, tmp.path(), TEMP_NAME)
                .await;
            // A damaged replica must not be copied: check the size and the
            // hash recorded when the file was stored before using it.
            let checked = match fetched {
                Ok(()) => crate::get::verify(&local, gap.size, gap.blake3.as_deref()).await,
                Err(e) => Err(e.to_string()),
            };
            match checked {
                Ok(()) => {
                    downloaded = true;
                    break;
                }
                Err(reason) => {
                    error = format!("{}: {reason}", account.label);
                    let _ = std::fs::remove_file(&local);
                }
            }
        }
        if !downloaded {
            report.failed.push((gap.path, error));
            continue;
        }

        for AccountId(slot) in targets {
            let target = &accounts[slot as usize];
            index.mark_pending(&gap.path, gap.size, &target.id).await?;
            match remote
                .upload(target, tmp.path(), TEMP_NAME, &gap.path)
                .await
            {
                Ok(()) => {
                    index.mark_done(&gap.path, &target.id).await?;
                    if let Some(s) = spaces.iter_mut().find(|s| s.id == AccountId(slot)) {
                        s.free_bytes = s.free_bytes.saturating_sub(gap.size);
                    }
                    report
                        .repaired
                        .push((gap.path.clone(), target.label.clone()));
                }
                Err(e) => report
                    .failed
                    .push((gap.path.clone(), format!("{}: {e}", target.label))),
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
    use std::collections::HashSet;
    use std::path::Path;

    struct Fake {
        free: HashMap<String, u64>,
        /// Accounts whose downloads fail.
        broken: HashSet<String>,
        uploads: RefCell<Vec<(String, String)>>,
        sizes: HashMap<String, u64>,
    }

    impl RemoteFs for Fake {
        async fn free_space(&self, a: &Account) -> Result<Option<u64>, RemoteError> {
            Ok(self.free.get(&a.id).copied())
        }
        async fn list_files(
            &self,
            _: &Account,
            _: &str,
        ) -> Result<crate::put::Listing, RemoteError> {
            Ok(crate::put::Listing::default())
        }
        async fn download(
            &self,
            a: &Account,
            remote_path: &str,
            dest_dir: &Path,
            dest_rel: &str,
        ) -> Result<(), RemoteError> {
            if self.broken.contains(&a.id) {
                return Err(RemoteError::Remote("quota".into()));
            }
            let len = usize::try_from(self.sizes[remote_path]).unwrap();
            std::fs::write(dest_dir.join(dest_rel), vec![1; len]).unwrap();
            Ok(())
        }
        async fn upload(
            &self,
            a: &Account,
            parent: &Path,
            name: &str,
            to: &str,
        ) -> Result<(), RemoteError> {
            assert!(parent.join(name).exists(), "uploads the downloaded file");
            self.uploads
                .borrow_mut()
                .push((a.id.clone(), to.to_owned()));
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

    fn mirror(copies: u8) -> Config {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Config::load(dir.path().join("config.toml")).unwrap();
        c.set(None, copies).unwrap();
        c
    }

    async fn setup(files: &[(&str, u64, &[&str])]) -> (Index, Fake) {
        let index = Index::open_memory().await.unwrap();
        let mut sizes = HashMap::new();
        for (path, size, holders) in files {
            sizes.insert((*path).to_owned(), *size);
            for h in *holders {
                index.mark_pending(path, *size, h).await.unwrap();
                index.mark_done(path, h).await.unwrap();
            }
        }
        let fake = Fake {
            free: ["a", "b", "c"]
                .iter()
                .map(|s| ((*s).to_owned(), 1000))
                .collect(),
            broken: HashSet::new(),
            uploads: RefCell::default(),
            sizes,
        };
        (index, fake)
    }

    #[tokio::test]
    async fn rebuilds_a_missing_copy_on_another_account() {
        let (index, fake) = setup(&[("x.bin", 10, &["a"])]).await;
        let accounts = [account("a"), account("b"), account("c")];
        let config = mirror(2);

        assert_eq!(
            find_gaps(&index, &accounts, &config, "")
                .await
                .unwrap()
                .len(),
            1
        );
        let r = repair(
            &fake,
            &index,
            &accounts,
            &MostFreeSpace::default(),
            &config,
            "",
        )
        .await
        .unwrap();
        assert!(r.is_complete());
        assert_eq!(r.repaired.len(), 1);
        assert_ne!(r.repaired[0].1, "a", "a distinct account");
        assert!(
            find_gaps(&index, &accounts, &config, "")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn copies_on_disconnected_accounts_do_not_count_and_total_loss_is_reported() {
        let (index, fake) = setup(&[("x.bin", 10, &["a", "gone"]), ("y.bin", 10, &["gone"])]).await;
        let accounts = [account("a"), account("b")];
        let r = repair(
            &fake,
            &index,
            &accounts,
            &MostFreeSpace::default(),
            &mirror(2),
            "",
        )
        .await
        .unwrap();
        assert_eq!(r.lost, ["y.bin"]);
        assert_eq!(r.repaired.len(), 1, "x.bin gets its second live copy on b");
    }

    #[tokio::test]
    async fn falls_back_to_another_holder_and_reports_no_room() {
        let (index, mut fake) = setup(&[("x.bin", 10, &["a", "b"])]).await;
        fake.broken.insert("a".into());
        let accounts = [account("a"), account("b"), account("c")];
        let r = repair(
            &fake,
            &index,
            &accounts,
            &MostFreeSpace::default(),
            &mirror(3),
            "",
        )
        .await
        .unwrap();
        assert_eq!(r.repaired, [("x.bin".to_owned(), "c".to_owned())]);

        // Wants 4 copies with only 3 accounts: the extra one has no room.
        let (index, fake) = setup(&[("z.bin", 10, &["a"])]).await;
        let r = repair(
            &fake,
            &index,
            &accounts,
            &MostFreeSpace::default(),
            &mirror(4),
            "",
        )
        .await
        .unwrap();
        assert_eq!(r.repaired.len(), 2, "every copy that fits is still made");
        assert_eq!(r.no_room, [("z.bin".to_owned(), 1)]);
    }

    #[tokio::test]
    async fn healthy_files_are_left_alone() {
        let (index, fake) = setup(&[("x.bin", 10, &["a", "b"])]).await;
        let accounts = [account("a"), account("b")];
        let r = repair(
            &fake,
            &index,
            &accounts,
            &MostFreeSpace::default(),
            &mirror(2),
            "",
        )
        .await
        .unwrap();
        assert!(r.repaired.is_empty() && r.is_complete());
        assert!(fake.uploads.borrow().is_empty());
    }

    #[tokio::test]
    async fn a_file_that_lost_every_copy_is_reported_lost_not_forgotten() {
        let (index, fake) = setup(&[("x.bin", 10, &["a"])]).await;
        // What `reconcile --apply` does when the only copy is gone remotely.
        index.remove_placement("x.bin", "a").await.unwrap();
        assert!(index.list("").await.unwrap().is_empty());

        let accounts = [account("a"), account("b")];
        let r = repair(
            &fake,
            &index,
            &accounts,
            &MostFreeSpace::default(),
            &mirror(1),
            "",
        )
        .await
        .unwrap();
        assert_eq!(r.lost, ["x.bin"]);
        assert!(!r.is_complete(), "a lost file must not read as success");
    }

    #[tokio::test]
    async fn a_damaged_replica_is_never_copied_to_a_new_account() {
        let (index, fake) = setup(&[("x.bin", 10, &["a", "b"])]).await;
        // Recorded as it was sent; the fake serves bytes of 1, which differ.
        let hash = blake3::hash(&[2u8; 10]).to_hex().to_string();
        index
            .mark_pending_hashed("x.bin", 10, "a", Some(&hash))
            .await
            .unwrap();
        index.mark_done("x.bin", "a").await.unwrap();

        let accounts = [account("a"), account("b"), account("c")];
        let r = repair(
            &fake,
            &index,
            &accounts,
            &MostFreeSpace::default(),
            &mirror(3),
            "",
        )
        .await
        .unwrap();
        assert!(r.repaired.is_empty(), "nothing damaged was propagated");
        assert!(r.failed[0].1.contains("hash"), "{:?}", r.failed);
        assert!(fake.uploads.borrow().is_empty());
    }

    #[tokio::test]
    async fn an_account_that_does_not_answer_is_set_aside_and_the_rest_still_repaired() {
        // "b" has no entry, so asking for its free space fails.
        let (index, mut fake) = setup(&[("x.bin", 10, &["a"])]).await;
        fake.free.remove("b");
        let accounts = [account("a"), account("b"), account("c")];
        let r = repair(
            &fake,
            &index,
            &accounts,
            &MostFreeSpace::default(),
            &mirror(2),
            "",
        )
        .await
        .unwrap();
        assert_eq!(r.repaired, [("x.bin".to_owned(), "c".to_owned())]);
        assert_eq!(r.unreachable.len(), 1);
        assert_eq!(r.unreachable[0].0, "b");
    }
}
