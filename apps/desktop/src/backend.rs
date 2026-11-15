//! Bridge between the UI and `bufonidae-core`. Core is async on tokio, the UI
//! runs on gpui's executor: work is spawned on a runtime owned here and the UI
//! awaits the join handle.

use std::sync::Arc;

use bufonidae_core::account::{Account, AccountStore};
use bufonidae_core::index::{self, Index, IndexedFile};
use bufonidae_core::manager::{self, SharedSecrets};
use bufonidae_core::secrets::KeyringStore;
use bufonidae_rclone::Error as RcloneError;
use bufonidae_rclone::rcd::Rcd;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

/// Why the app could not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupError {
    /// Another Bufonidae process holds the index.
    InUse,
    Other(String),
}

/// What the screens show: accounts and every stored file.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub accounts: Vec<Account>,
    pub files: Vec<IndexedFile>,
}

impl Snapshot {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// `(files, bytes)` stored on the account with this id.
    pub fn usage(&self, account_id: &str) -> (usize, u64) {
        self.files
            .iter()
            .filter(|f| f.accounts.iter().any(|a| a == account_id))
            .fold((0, 0), |(n, b), f| (n + 1, b + f.size))
    }
}

pub struct Backend {
    runtime: Runtime,
    index: Arc<Index>,
}

impl Backend {
    /// Starts the runtime and opens the index. Fails with
    /// [`StartupError::InUse`] when another process has it open.
    pub fn open() -> Result<Self, StartupError> {
        let other = |e: &dyn std::fmt::Display| StartupError::Other(e.to_string());
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| other(&e))?;
        let path = Index::default_path().map_err(|e| other(&e))?;
        let index = runtime.block_on(Index::open(&path)).map_err(|e| match e {
            index::Error::InUse => StartupError::InUse,
            e => other(&e),
        })?;
        Ok(Self {
            runtime,
            index: Arc::new(index),
        })
    }

    /// Reads accounts from disk and files from the index, off the UI thread.
    pub fn load(&self) -> JoinHandle<Result<Snapshot, String>> {
        let index = Arc::clone(&self.index);
        self.runtime.spawn(async move {
            let path = AccountStore::default_path().map_err(|e| e.to_string())?;
            let accounts = AccountStore::load(path)
                .map_err(|e| e.to_string())?
                .list()
                .to_vec();
            let files = index.list("").await.map_err(|e| e.to_string())?;
            Ok(Snapshot { accounts, files })
        })
    }

    /// Connects a Mega account: proves the login through rclone, then stores
    /// the account and its secrets. Nothing is kept when it fails.
    pub fn add_account(&self, request: NewAccount) -> JoinHandle<Result<(), String>> {
        self.runtime.spawn(async move {
            let path = AccountStore::default_path().map_err(|e| e.to_string())?;
            let mut store = AccountStore::load(path).map_err(|e| e.to_string())?;
            // Cheap checks first, so a typo does not wait for a login attempt.
            store
                .prepare(&request.label, &request.email)
                .map_err(|e| e.to_string())?;

            let rcd = Rcd::start("rclone").await.map_err(|e| match e {
                RcloneError::Spawn(io) if io.kind() == std::io::ErrorKind::NotFound => {
                    "rclone was not found. Install it and make sure it is on your PATH.".to_owned()
                }
                e => e.to_string(),
            })?;
            let secrets: SharedSecrets = Arc::new(KeyringStore);
            let result = manager::add_account(
                &rcd,
                &mut store,
                &secrets,
                &request.label,
                &request.email,
                &request.password,
                request.two_fa.as_deref(),
            )
            .await;
            let _ = rcd.shutdown().await;
            result.map(|_| ()).map_err(|e| e.to_string())
        })
    }
}

/// What the "add account" form submits. The password is only held until the
/// login attempt finishes.
pub struct NewAccount {
    pub label: String,
    pub email: String,
    pub password: String,
    pub two_fa: Option<String>,
}

/// A label for the account when the user leaves the name empty: the part of
/// the e-mail before the `@`, reduced to the characters labels accept.
pub fn suggest_label(email: &str) -> String {
    let local: String = email
        .split('@')
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .take(32)
        .collect();
    if local.is_empty() {
        "account".to_owned()
    } else {
        local
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_a_valid_label_from_the_email() {
        assert_eq!(suggest_label("ana.silva@example.com"), "ana.silva");
        assert_eq!(suggest_label("a b+c@example.com"), "abc");
        assert_eq!(suggest_label("@example.com"), "account");
        assert_eq!(suggest_label(""), "account");
        assert_eq!(suggest_label(&format!("{}@x.y", "z".repeat(50))).len(), 32);
    }
}
