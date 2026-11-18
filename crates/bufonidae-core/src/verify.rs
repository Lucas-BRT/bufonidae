//! Explicit deep verification: downloads each indexed replica without changing
//! remote files or establishing a new trusted hash from untrusted content.

use crate::{
    account::Account,
    get,
    index::{self, Index},
    paths::is_safe_stored_path,
    put::RemoteFs,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Index(#[from] index::Error),
    #[error("verification temporary folder: {0}")]
    Temp(#[from] std::io::Error),
}

#[derive(Debug, Default)]
pub struct Report {
    pub verified: usize,
    /// Known-size bytes successfully fetched for verification.
    pub bytes: u64,
    /// No baseline exists; neither the content nor its replicas are blessed.
    pub without_hash: Vec<String>,
    /// Path, account id, reason. Includes disconnected accounts.
    pub failed: Vec<(String, String, String)>,
}

/// Verifies every confirmed replica under a folder. One local temporary file
/// is retained at a time. Dropping this future removes its temporary folder.
pub async fn verify<R: RemoteFs>(
    remote: &R,
    index: &Index,
    accounts: &[Account],
    prefix: &str,
) -> Result<Report, Error> {
    let mut report = Report::default();
    let prefix = prefix.trim_matches('/');
    let staging = tempfile::tempdir()?;
    for file in index
        .list(prefix)
        .await?
        .into_iter()
        .filter(|f| get::is_under(&f.path, prefix))
    {
        let Some(hash) = file.blake3.as_deref() else {
            report.without_hash.push(file.path);
            continue;
        };
        for id in &file.accounts {
            if !is_safe_stored_path(&file.path) {
                report
                    .failed
                    .push((file.path.clone(), id.clone(), "unsafe indexed path".into()));
                continue;
            }
            let Some(account) = accounts.iter().find(|a| &a.id == id) else {
                report.failed.push((
                    file.path.clone(),
                    id.clone(),
                    "account is not connected".into(),
                ));
                continue;
            };
            // A separate private directory for each replica also ensures that
            // a failed download can never leave bytes used by the next check.
            let replica = tempfile::tempdir_in(staging.path())?;
            let result = match remote
                .download(account, &file.path, replica.path(), "content")
                .await
            {
                Ok(()) => get::verify(&replica.path().join("content"), file.size, Some(hash)).await,
                Err(error) => Err(error.to_string()),
            };
            match result {
                Ok(()) => {
                    report.verified += 1;
                    report.bytes = report.bytes.saturating_add(file.size);
                }
                Err(reason) => report.failed.push((file.path.clone(), id.clone(), reason)),
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::put::{Error as RemoteError, Listing};
    use std::path::Path;

    struct Fake;
    impl RemoteFs for Fake {
        async fn free_space(&self, _: &Account) -> Result<Option<u64>, RemoteError> {
            unreachable!()
        }
        async fn list_files(&self, _: &Account, _: &str) -> Result<Listing, RemoteError> {
            unreachable!()
        }
        async fn upload(&self, _: &Account, _: &Path, _: &str, _: &str) -> Result<(), RemoteError> {
            unreachable!()
        }
        async fn download(
            &self,
            account: &Account,
            _: &str,
            dir: &Path,
            name: &str,
        ) -> Result<(), RemoteError> {
            std::fs::write(
                dir.join(name),
                if account.id == "good" {
                    b"good"
                } else {
                    b"evil"
                },
            )
            .unwrap();
            Ok(())
        }
    }

    #[tokio::test]
    async fn checks_every_replica_and_does_not_bless_missing_hashes() {
        let index = Index::open_memory().await.unwrap();
        let hash = blake3::hash(b"good").to_hex().to_string();
        for id in ["good", "bad", "offline"] {
            index
                .mark_pending_hashed("Docs/file", 4, id, Some(&hash))
                .await
                .unwrap();
            index.mark_done("Docs/file", id).await.unwrap();
        }
        index.mark_pending("Docs/unknown", 4, "good").await.unwrap();
        index.mark_done("Docs/unknown", "good").await.unwrap();
        // Literal folder boundaries: this adjacent folder must not be checked.
        index
            .mark_pending("Docs-other/file", 4, "good")
            .await
            .unwrap();
        index.mark_done("Docs-other/file", "good").await.unwrap();
        let accounts = ["good", "bad"].map(|id| Account {
            id: id.into(),
            label: id.into(),
            email: format!("{id}@example.com"),
        });
        let before = index.placements("").await.unwrap();
        let report = verify(&Fake, &index, &accounts, "Docs").await.unwrap();
        assert_eq!(report.verified, 1);
        assert_eq!(report.bytes, 4);
        assert_eq!(report.without_hash, ["Docs/unknown"]);
        assert_eq!(report.failed.len(), 2);
        assert!(
            report
                .failed
                .iter()
                .any(|(_, id, error)| id == "bad" && error.contains("hash"))
        );
        assert!(report.failed.iter().any(|(_, id, _)| id == "offline"));
        assert_eq!(index.placements("").await.unwrap(), before);
    }
}
