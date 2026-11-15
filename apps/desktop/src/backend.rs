//! Bridge between the UI and `bufonidae-core`. Core is async on tokio, the UI
//! runs on gpui's executor: work is spawned on a runtime owned here and the UI
//! awaits the join handle.

use std::sync::Arc;

use bufonidae_core::account::{Account, AccountStore};
use bufonidae_core::index::{self, Index, IndexedFile};
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
}
