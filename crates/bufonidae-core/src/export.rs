//! A portable backup of everything Bufonidae knows that a remote listing
//! cannot recreate: which accounts hold which files, the redundancy settings,
//! and the files that were lost.
//!
//! The file is plain JSON, independent of the database that holds the index.
//! It never contains a password, a session or a master key: after an import,
//! accounts must sign in again.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::account::{self, Account, AccountStore};
use crate::config::{self, Config};
use crate::index::{self, Index, LostFile, Placement, State};

/// Version of the file layout. Bump it when the layout changes.
pub const FORMAT: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Index(#[from] index::Error),
    #[error(transparent)]
    Account(#[from] account::Error),
    #[error(transparent)]
    Config(#[from] config::Error),
    #[error("{path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("not a Bufonidae export, or damaged: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("this export uses format {found}, but this build reads format {FORMAT}")]
    UnsupportedFormat { found: u32 },
    #[error("the export is inconsistent: {0}")]
    Inconsistent(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportedPlacement {
    pub path: String,
    pub size: u64,
    pub account_id: String,
    /// `"done"` or `"pending"`.
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportedLost {
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Export {
    pub format: u32,
    /// The Bufonidae version that wrote it, for diagnosing problems.
    pub written_by: String,
    pub accounts: Vec<Account>,
    pub config: Config,
    pub placements: Vec<ExportedPlacement>,
    pub lost: Vec<ExportedLost>,
    /// BLAKE3 of files, by path. Absent in backups made before hashes existed.
    #[serde(default)]
    pub hashes: std::collections::BTreeMap<String, String>,
}

/// Reads everything into one value. Secrets are not part of it.
pub async fn collect(
    index: &Index,
    accounts: &AccountStore,
    config: &Config,
) -> Result<Export, Error> {
    let placements = index
        .placements("")
        .await?
        .into_iter()
        .map(|p| ExportedPlacement {
            path: p.path,
            size: p.size,
            account_id: p.account_id,
            state: match p.state {
                State::Done => "done",
                State::Pending => "pending",
            }
            .to_owned(),
        })
        .collect();
    let lost = index
        .lost("")
        .await?
        .into_iter()
        .map(|l| ExportedLost {
            path: l.path,
            size: l.size,
        })
        .collect();
    let hashes = index.hashes("").await?.into_iter().collect();
    Ok(Export {
        format: FORMAT,
        written_by: env!("CARGO_PKG_VERSION").to_owned(),
        accounts: accounts.list().to_vec(),
        config: config.clone(),
        placements,
        lost,
        hashes,
    })
}

/// Writes the export privately and atomically.
pub fn write(path: &Path, export: &Export) -> Result<(), Error> {
    let text = serde_json::to_string_pretty(export)?;
    crate::fsutil::write_private(path, &text).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })
}

/// Reads and checks an export file.
pub fn read(path: &Path) -> Result<Export, Error> {
    let text = std::fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })?;
    let export: Export = serde_json::from_str(&text)?;
    if export.format != FORMAT {
        return Err(Error::UnsupportedFormat {
            found: export.format,
        });
    }
    validate(&export)?;
    Ok(export)
}

fn safe_path(path: &str) -> bool {
    crate::paths::is_safe_stored_path(path)
}

fn validate(export: &Export) -> Result<(), Error> {
    let bad = |message: String| Err(Error::Inconsistent(message));
    let mut size_of = std::collections::HashMap::new();
    for p in &export.placements {
        if !safe_path(&p.path) {
            return bad(format!("unsafe path {:?}", p.path));
        }
        if p.state != "done" && p.state != "pending" {
            return bad(format!("{}: unknown state {:?}", p.path, p.state));
        }
        if !export.accounts.iter().any(|a| a.id == p.account_id) {
            return bad(format!("{}: unknown account {:?}", p.path, p.account_id));
        }
        if *size_of.entry(p.path.as_str()).or_insert(p.size) != p.size {
            return bad(format!("{}: copies disagree on the size", p.path));
        }
    }
    for l in &export.lost {
        if !safe_path(&l.path) {
            return bad(format!("unsafe path {:?}", l.path));
        }
    }
    for (path, hash) in &export.hashes {
        let known =
            size_of.contains_key(path.as_str()) || export.lost.iter().any(|l| &l.path == path);
        if !known {
            return bad(format!(
                "{path}: a hash for a file that is not in the export"
            ));
        }
        if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return bad(format!("{path}: not a BLAKE3 hash"));
        }
    }
    Ok(())
}

/// Restores an export into an **empty** index, an empty account list and the
/// given config, all or nothing as far as the index is concerned. Accounts
/// come back without secrets and must sign in again.
pub async fn restore(
    export: &Export,
    index: &Index,
    accounts: &mut AccountStore,
    config: &mut Config,
) -> Result<(), Error> {
    validate(export)?;
    let rows: Vec<Placement> = export
        .placements
        .iter()
        .map(|p| Placement {
            path: p.path.clone(),
            size: p.size,
            account_id: p.account_id.clone(),
            state: if p.state == "done" {
                State::Done
            } else {
                State::Pending
            },
        })
        .collect();
    let lost: Vec<LostFile> = export
        .lost
        .iter()
        .map(|l| LostFile {
            path: l.path.clone(),
            size: l.size,
        })
        .collect();
    // Checked first: both refuse to mix with existing data, and the index
    // import is the only step that is hard to undo.
    if !accounts.list().is_empty() {
        return Err(account::Error::NotEmpty.into());
    }
    let hashes = export.hashes.clone().into_iter().collect();
    index.import(&rows, &lost, &hashes).await?;
    accounts.restore(export.accounts.clone())?;
    config.restore_from(&export.config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Profile {
        _dir: tempfile::TempDir,
        index: Index,
        accounts: AccountStore,
        config: Config,
    }

    async fn profile() -> Profile {
        let dir = tempfile::tempdir().unwrap();
        let index = Index::open_memory().await.unwrap();
        let accounts = AccountStore::load(dir.path().join("accounts.toml")).unwrap();
        let config = Config::load(dir.path().join("config.toml")).unwrap();
        Profile {
            _dir: dir,
            index,
            accounts,
            config,
        }
    }

    async fn populated() -> Profile {
        let mut p = profile().await;
        let a = p.accounts.add("one", "one@example.com").unwrap();
        let b = p.accounts.add("two", "two@example.com").unwrap();
        p.config.set(None, 2).unwrap();
        p.config.set(Some("Docs"), 3).unwrap();
        p.config.set_large_upload_bytes(5_000_000_000).unwrap();
        let hash = "a".repeat(64);
        for acct in [&a.id, &b.id] {
            p.index
                .mark_pending_hashed("Pics/a.jpg", 100, acct, Some(&hash))
                .await
                .unwrap();
            p.index.mark_done("Pics/a.jpg", acct).await.unwrap();
        }
        // Unfinished, and a lost file.
        p.index
            .mark_pending("Pics/half.bin", 7, &a.id)
            .await
            .unwrap();
        p.index.mark_pending("gone.bin", 9, &a.id).await.unwrap();
        p.index.mark_done("gone.bin", &a.id).await.unwrap();
        p.index.remove_placement("gone.bin", &a.id).await.unwrap();
        p
    }

    #[tokio::test]
    async fn a_full_export_restores_into_a_fresh_profile() {
        let src = populated().await;
        let export = collect(&src.index, &src.accounts, &src.config)
            .await
            .unwrap();

        // Through a file, as a user would.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("backup.json");
        write(&file, &export).unwrap();
        let loaded = read(&file).unwrap();
        assert_eq!(loaded, export);

        let mut fresh = profile().await;
        restore(
            &loaded,
            &fresh.index,
            &mut fresh.accounts,
            &mut fresh.config,
        )
        .await
        .unwrap();

        assert_eq!(fresh.accounts.list(), src.accounts.list(), "ids are kept");
        assert_eq!(fresh.config.copies_for("Docs/x"), 3);
        assert_eq!(fresh.config.copies_for("Other"), 2);
        assert_eq!(fresh.config.large_upload_bytes(), 5_000_000_000);
        assert_eq!(
            fresh.index.placements("").await.unwrap(),
            src.index.placements("").await.unwrap()
        );
        assert_eq!(
            fresh.index.lost("").await.unwrap(),
            src.index.lost("").await.unwrap()
        );
        assert_eq!(
            fresh.index.hashes("").await.unwrap().get("Pics/a.jpg"),
            Some(&"a".repeat(64)),
            "the hash travels with the backup"
        );
    }

    #[tokio::test]
    async fn the_export_file_is_private_and_holds_no_secret() {
        use std::os::unix::fs::PermissionsExt as _;
        let src = populated().await;
        let export = collect(&src.index, &src.accounts, &src.config)
            .await
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("backup.json");
        write(&file, &export).unwrap();

        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let text = std::fs::read_to_string(&file).unwrap().to_lowercase();
        for word in ["password", "session", "master", "secret"] {
            assert!(!text.contains(word), "export mentions {word}");
        }
    }

    #[tokio::test]
    async fn restoring_refuses_to_mix_with_existing_data() {
        let src = populated().await;
        let export = collect(&src.index, &src.accounts, &src.config)
            .await
            .unwrap();

        // A non-empty index.
        let mut busy = profile().await;
        busy.index.mark_pending("x", 1, "a").await.unwrap();
        assert!(
            restore(&export, &busy.index, &mut busy.accounts, &mut busy.config)
                .await
                .is_err()
        );
        assert!(busy.accounts.list().is_empty(), "nothing else was touched");

        // A non-empty account list.
        let mut has_accounts = profile().await;
        has_accounts
            .accounts
            .add("mine", "mine@example.com")
            .unwrap();
        assert!(
            restore(
                &export,
                &has_accounts.index,
                &mut has_accounts.accounts,
                &mut has_accounts.config
            )
            .await
            .is_err()
        );
        assert!(has_accounts.index.placements("").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn damaged_or_inconsistent_exports_are_rejected_before_anything_changes() {
        let src = populated().await;
        let good = collect(&src.index, &src.accounts, &src.config)
            .await
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("b.json");

        let mut cases: Vec<(&str, Export)> = Vec::new();
        let mut e = good.clone();
        e.placements[0].path = "../escape".into();
        cases.push(("unsafe path", e));
        let mut e = good.clone();
        e.placements[0].account_id = "nobody".into();
        cases.push(("unknown account", e));
        let mut e = good.clone();
        e.placements[0].state = "weird".into();
        cases.push(("unknown state", e));
        let mut e = good.clone();
        e.placements[1].size += 1; // both copies of Pics/a.jpg must agree
        cases.push(("size disagreement", e));
        let mut e = good.clone();
        e.hashes.insert("Pics/a.jpg".into(), "not-a-hash".into());
        cases.push(("malformed hash", e));
        let mut e = good.clone();
        e.hashes.insert("ghost.bin".into(), "b".repeat(64));
        cases.push(("hash for an unknown file", e));
        let mut e = good.clone();
        e.format = 99;
        cases.push(("future format", e));
        for (name, export) in cases {
            write(&file, &export).unwrap();
            assert!(read(&file).is_err(), "{name} was accepted");
        }
        std::fs::write(&file, "{ not json").unwrap();
        assert!(matches!(read(&file), Err(Error::Parse(_))));
    }

    /// What the index does when its file is garbage: a clear error, no panic,
    /// and the damaged file is left exactly as it was for the user to keep.
    #[tokio::test]
    async fn a_corrupted_index_file_fails_clearly_and_is_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        std::fs::write(
            &path,
            b"this is definitely not a database file at all, just text",
        )
        .unwrap();
        let before = std::fs::read(&path).unwrap();

        let result = Index::open(&path).await;
        assert!(result.is_err(), "garbage was accepted as an index");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "the file was modified"
        );
    }
}
