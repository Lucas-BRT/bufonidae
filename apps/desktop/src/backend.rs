//! Bridge between the UI and `bufonidae-core`. Core is async on tokio, the UI
//! runs on gpui's executor: work is spawned on a runtime owned here and the UI
//! awaits the join handle.

use std::sync::Arc;

mod sync;
pub use sync::SyncPreview;

use bufonidae_core::account::{Account, AccountStore};
use bufonidae_core::config::Config;
use bufonidae_core::get;
use bufonidae_core::index::{self, Index, IndexedFile};
use bufonidae_core::manager::{self, SharedSecrets};
use bufonidae_core::placement::MostFreeSpace;
use bufonidae_core::put::{self, Progress};
use bufonidae_core::reconcile;
use bufonidae_core::repair;
use bufonidae_core::secrets::KeyringStore;
use bufonidae_rclone::Error as RcloneError;
use bufonidae_rclone::rcd::Rcd;
use std::path::PathBuf;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
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
    /// Copies whose upload started and was never confirmed (an interrupted
    /// store). `reconcile` settles them.
    pub pending: usize,
    pub jobs: Vec<index::Job>,
}

impl Snapshot {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// How many files have a copy on this account and on no other.
    pub fn only_on(&self, account_id: &str) -> usize {
        self.files
            .iter()
            .filter(|f| f.accounts.len() == 1 && f.accounts[0] == account_id)
            .count()
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
    pub(crate) tray: Option<crate::tray::Service>,
    runtime: Runtime,
    index: Arc<Index>,
    sessions: Arc<tokio::sync::Mutex<SessionCache>>,
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
        runtime
            .block_on(index.interrupt_active_jobs())
            .map_err(|e| other(&e))?;
        let index = Arc::new(index);
        let tray = Some(crate::tray::Service::new(&runtime, Arc::clone(&index)));
        Ok(Self {
            tray,
            runtime,
            index,
            sessions: Arc::new(tokio::sync::Mutex::new(SessionCache::default())),
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
            let pending = index.pending().await.map_err(|e| e.to_string())?.len();
            let jobs = index.jobs().await.map_err(|e| e.to_string())?;
            Ok(Snapshot {
                accounts,
                files,
                pending,
                jobs,
            })
        })
    }

    pub fn load_jobs(&self) -> JoinHandle<Result<Vec<index::Job>, String>> {
        let index = Arc::clone(&self.index);
        self.runtime
            .spawn(async move { index.jobs().await.map_err(|e| e.to_string()) })
    }

    /// Looks at what was dropped and at the settings, without touching Mega.
    pub fn inspect(&self, paths: Vec<PathBuf>) -> JoinHandle<Result<Inspection, String>> {
        self.runtime.spawn(async move {
            let roots = paths.clone();
            let scan = tokio::task::spawn_blocking(move || put::scan(&paths))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            let accounts =
                AccountStore::load(AccountStore::default_path().map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?
                    .list()
                    .len();
            let config = Config::load(Config::default_path().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            let home = home_dir();
            let includes_home = home.as_ref().is_some_and(|home| {
                roots
                    .iter()
                    .any(|root| std::fs::canonicalize(root).is_ok_and(|r| covers(&r, home)))
            });
            Ok(Inspection {
                files: scan.files.len(),
                bytes: scan.files.iter().map(|f| f.size).sum(),
                skipped: scan.skipped.len(),
                accounts,
                includes_home,
                config,
            })
        })
    }

    /// Stores `paths` under `dest` on the connected accounts, following the
    /// redundancy settings. Progress arrives on `events` as it happens.
    pub fn store(&self, paths: Vec<PathBuf>, dest: String) -> StoreJob {
        let (tx, events) = unbounded_channel();
        let (cancel, cancel_rx) = tokio::sync::oneshot::channel::<()>();
        let index = Arc::clone(&self.index);
        let sessions = Arc::clone(&self.sessions);
        let result = self.runtime.spawn(async move {
            let payload = serde_json::json!({
                "paths": serde_json::to_value(&paths).map_err(|e| e.to_string())?,
                "destination": dest,
            })
            .to_string();
            let job_id = index
                .enqueue_job("upload", &payload)
                .await
                .map_err(|e| e.to_string())?;
            let _ = tx.send(TransferEvent::Stage("Waiting to start"));
            let cancelled = async {
                if cancel_rx.await.is_err() { std::future::pending::<()>().await; }
            };
            tokio::pin!(cancelled);
            let mut cache = tokio::select! {
                biased;
                () = &mut cancelled => {
                    index.set_job_state(&job_id, "cancelled", None).await.map_err(|e| e.to_string())?;
                    return Ok(StoreOutcome::Cancelled);
                }
                cache = sessions.lock() => cache,
            };
            index
                .set_job_state(&job_id, "running", None)
                .await
                .map_err(|e| e.to_string())?;
            let outcome = async {
                let store =
                    AccountStore::load(AccountStore::default_path().map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?;
                let accounts = store.list().to_vec();
                if accounts.is_empty() {
                    return Err("Connect a Mega account first.".to_owned());
                }
                let config = Config::load(Config::default_path().map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;

                let _ = tx.send(TransferEvent::Stage("Connecting accounts"));
                let connection = tokio::select! {
                    biased;
                    () = &mut cancelled => return Ok(StoreOutcome::Cancelled),
                    connection = cache.checkout(&store) => connection?,
                };
                let _ = tx.send(TransferEvent::Stage("Planning files and checking destinations"));
                let Connected {
                    rcd,
                    usable,
                    unreachable,
                } = &connection;
                let accounts = usable;
                // The daemon lease is exclusive: resetting counters cannot
                // interfere with another transfer.
                let live = tokio::time::timeout(std::time::Duration::from_secs(2), rcd.reset_transfer_stats()).await.is_ok_and(|result| result.is_ok());
                if !live { let _ = tx.send(TransferEvent::Live(None)); }

                let outcome = async {
                    let policy = MostFreeSpace::default();
                    // The receiver may be gone (window closed); the store goes on.
                    let mut observer = |progress| {
                        if matches!(progress, Progress::Planned { .. }) {
                            let _ = tx.send(TransferEvent::Stage("Transferring files"));
                        }
                        let _ = tx.send(TransferEvent::Progress(progress));
                    };
                    let store = put::put_observed(
                        rcd,
                        &index,
                        accounts,
                        &policy,
                        &config,
                        &paths,
                        &dest,
                        &mut observer,
                    );
                    tokio::pin!(store);
                    let telemetry = observe_transfer_bytes(rcd, &tx, live);
                    tokio::pin!(telemetry);
                    tokio::select! {
                        () = &mut telemetry => Err("Transfer monitor stopped unexpectedly.".to_owned()),
                        report = &mut store => report
                            .map(|mut r| {
                                // Accounts that could not even sign in.
                                r.unreachable.extend(unreachable.iter().cloned());
                                StoreOutcome::Done(r)
                            })
                            .map_err(|e| e.to_string()),
                        () = &mut cancelled => Ok(StoreOutcome::Cancelled),
                    }
                }
                .await;
                // The cache retains successful sessions and stops cancelled transfers
                // before another operation can lease the daemon.
                cache
                    .release(connection, matches!(&outcome, Ok(StoreOutcome::Done(_))))
                    .await;
                outcome
            }
            .await;
            let (state, reason) = match &outcome {
                Ok(StoreOutcome::Done(report)) if report.is_complete() => ("done", None),
                Ok(StoreOutcome::Done(report)) => {
                    let mut reasons = report.failed.iter().chain(&report.differs).map(|(path, error)| format!("{path}: {error}")).collect::<Vec<_>>();
                    reasons.extend(report.no_room.iter().map(|path| format!("{path}: not enough free space for every copy")));
                    ("failed", Some(reasons.join("\n")))
                }
                Ok(StoreOutcome::Cancelled) => ("cancelled", None),
                Err(message) => ("failed", Some(message.clone())),
            };
            index
                .set_job_state(&job_id, state, reason.as_deref())
                .await
                .map_err(|e| e.to_string())?;
            let failed=state=="failed";
            let cancelled=state=="cancelled";
            let saved_reason=reason.clone();
            let state=state.to_owned();
            let detail=payload.clone();
            tokio::spawn(async move {
                if let Err(error)=crate::notifications::announce(failed,cancelled,&detail).await {
                    let detail=format!("{}Desktop notification: {error}",saved_reason.map(|reason|format!("{reason}\n")).unwrap_or_default());
                    let _=index.set_job_state(&job_id,&state,Some(&detail)).await;
                }
            });
            outcome
        });
        StoreJob {
            events,
            cancel,
            result,
        }
    }

    /// Downloads `path` (a file or a folder, as stored) into `out`, following
    /// the core's rules: nothing local is overwritten, sizes are checked.
    pub fn download_selection(&self, paths: Vec<String>, out: PathBuf) -> DownloadJob {
        let (tx, events) = unbounded_channel();
        let (cancel, cancel_rx) = tokio::sync::oneshot::channel::<()>();
        let index = Arc::clone(&self.index);
        let sessions = Arc::clone(&self.sessions);
        let result = self.runtime.spawn(async move {
            let payload =
                serde_json::to_string(&(paths.clone(), out.clone())).map_err(|e| e.to_string())?;
            let job_id = index
                .enqueue_job("download", &payload)
                .await
                .map_err(|e| e.to_string())?;
            let _ = tx.send(TransferEvent::Stage("Waiting to start"));
            let cancelled = async {
                if cancel_rx.await.is_err() { std::future::pending::<()>().await; }
            };
            tokio::pin!(cancelled);
            let mut cache = tokio::select! {
                biased;
                () = &mut cancelled => {
                    index.set_job_state(&job_id, "cancelled", None).await.map_err(|e| e.to_string())?;
                    return Ok(DownloadOutcome::Cancelled);
                }
                cache = sessions.lock() => cache,
            };
            index
                .set_job_state(&job_id, "running", None)
                .await
                .map_err(|e| e.to_string())?;
            let outcome = async {
                let store =
                    AccountStore::load(AccountStore::default_path().map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?;
                let accounts = store.list().to_vec();
                if accounts.is_empty() {
                    return Err("Connect a Mega account first.".to_owned());
                }
                let _ = tx.send(TransferEvent::Stage("Connecting accounts"));
                let connection = tokio::select! {
                    biased;
                    () = &mut cancelled => return Ok(DownloadOutcome::Cancelled),
                    connection = cache.checkout(&store) => connection?,
                };
                let _ = tx.send(TransferEvent::Stage("Planning files and checking destinations"));
                let Connected {
                    rcd,
                    usable,
                    unreachable,
                } = &connection;
                let accounts = usable;
                // The daemon lease is exclusive: resetting counters cannot
                // interfere with another transfer.
                let live = tokio::time::timeout(std::time::Duration::from_secs(2), rcd.reset_transfer_stats()).await.is_ok_and(|result| result.is_ok());
                if !live { let _ = tx.send(TransferEvent::Live(None)); }

                let outcome = async {
                    let mut observer = |progress| {
                        if matches!(progress, get::Progress::Planned { .. }) {
                            let _ = tx.send(TransferEvent::Stage("Transferring files"));
                        }
                        let _ = tx.send(TransferEvent::Progress(progress));
                    };
                    let download = async {
                        if let [path]=paths.as_slice() { get::get_observed(rcd,&index,accounts,path,&out,&mut observer).await }
                        else {get::get_selected_observed(rcd,&index,accounts,&paths,&out,&mut observer).await}
                    };
                    tokio::pin!(download);
                    let telemetry = observe_transfer_bytes(rcd, &tx, live);
                    tokio::pin!(telemetry);
                    tokio::select! {
                        () = &mut telemetry => Err("Transfer monitor stopped unexpectedly.".to_owned()),
                        report = &mut download => {
                            report
                                .map(|mut r| {
                                    r.unreachable.extend(unreachable.iter().cloned());
                                    DownloadOutcome::Done(r)
                                })
                                .map_err(|e| e.to_string())
                        }
                        () = &mut cancelled => Ok(DownloadOutcome::Cancelled),
                    }
                }
                .await;
                cache
                    .release(connection, matches!(&outcome, Ok(DownloadOutcome::Done(_))))
                    .await;
                outcome
            }
            .await;
            let (state, reason) = match &outcome {
                Ok(DownloadOutcome::Done(report)) if report.is_complete() => ("done", None),
                Ok(DownloadOutcome::Done(report)) => {
                    let reasons = report.failed.iter().map(|(path, error)| format!("{path}: {error}")).collect::<Vec<_>>();
                    ("failed", Some(reasons.join("\n")))
                }
                Ok(DownloadOutcome::Cancelled) => ("cancelled", None),
                Err(message) => ("failed", Some(message.clone())),
            };
            index
                .set_job_state(&job_id, state, reason.as_deref())
                .await
                .map_err(|e| e.to_string())?;
            let failed=state=="failed";
            let cancelled=state=="cancelled";
            let saved_reason=reason.clone();
            let state=state.to_owned();
            let detail=payload.clone();
            tokio::spawn(async move {
                if let Err(error)=crate::notifications::announce(failed,cancelled,&detail).await {
                    let detail=format!("{}Desktop notification: {error}",saved_reason.map(|reason|format!("{reason}\n")).unwrap_or_default());
                    let _=index.set_job_state(&job_id,&state,Some(&detail)).await;
                }
            });
            outcome
        });
        DownloadJob {
            events,
            cancel,
            result,
        }
    }

    /// Lists a folder for the file browser, off the UI thread.
    pub fn list_dir(&self, dir: PathBuf) -> JoinHandle<Result<Vec<DirEntryInfo>, String>> {
        self.runtime.spawn(async move {
            tokio::task::spawn_blocking(move || read_dir_sorted(&dir))
                .await
                .map_err(|e| e.to_string())?
        })
    }

    /// Reads the settings from disk.
    pub fn load_config(&self) -> JoinHandle<Result<Config, String>> {
        self.runtime.spawn(async move {
            Config::load(Config::default_path().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())
        })
    }

    /// Changes the redundancy settings and returns the saved result.
    pub fn change_redundancy(
        &self,
        change: RedundancyChange,
    ) -> JoinHandle<Result<Config, String>> {
        self.runtime.spawn(async move {
            let mut config = Config::load(Config::default_path().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            match change {
                RedundancyChange::Default(copies) => config.set(None, copies),
                RedundancyChange::Rule { folder, copies } => config.set(Some(&folder), copies),
                RedundancyChange::RemoveRule(folder) => config.unset(&folder),
            }
            .map_err(|e| e.to_string())?;
            Ok(config)
        })
    }

    /// Saves the "ask before storing more than this" limit.
    pub fn save_large_upload_bytes(&self, bytes: u64) -> JoinHandle<Result<Config, String>> {
        self.runtime.spawn(async move {
            let path = Config::default_path().map_err(|e| e.to_string())?;
            let mut config = Config::load(path).map_err(|e| e.to_string())?;
            config
                .set_large_upload_bytes(bytes)
                .map_err(|e| e.to_string())?;
            Ok(config)
        })
    }

    pub fn save_reduced_motion(&self, reduced: bool) -> JoinHandle<Result<Config, String>> {
        self.runtime.spawn(async move {
            let mut config = Config::load(Config::default_path().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            config
                .set_reduced_motion(reduced)
                .map_err(|e| e.to_string())?;
            Ok(config)
        })
    }

    pub fn save_desktop_options(
        &self,
        options: bufonidae_core::config::DesktopOptions,
    ) -> JoinHandle<Result<Config, String>> {
        self.runtime.spawn(async move {
            let mut config = Config::load(Config::default_path().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            config.set_desktop(options).map_err(|e| e.to_string())?;
            Ok(config)
        })
    }

    pub fn test_notification(
        &self,
        options: bufonidae_core::config::DesktopOptions,
        failed: bool,
    ) -> JoinHandle<Result<(), String>> {
        self.runtime.spawn(async move {
            crate::notifications::deliver(
                &options,
                "Bufonidae notification test",
                "This test uses your unsaved notification and sound preferences.",
                failed,
            )
            .await
        })
    }

    /// Compares the index with what the accounts really hold. With `apply`
    /// false nothing is changed: it only reports.
    pub fn check_files(&self, apply: bool) -> JoinHandle<Result<CheckOutcome, String>> {
        let index = Arc::clone(&self.index);
        let sessions = Arc::clone(&self.sessions);
        self.runtime.spawn(async move {
            let mut cache = sessions.lock().await;
            let store =
                AccountStore::load(AccountStore::default_path().map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            if store.list().is_empty() {
                return Err("Connect a Mega account first.".to_owned());
            }
            let config = Config::load(Config::default_path().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            let connection = cache.checkout(&store).await?;
            let Connected {
                rcd,
                usable,
                unreachable,
            } = &connection;
            let outcome = async {
                let mut report = reconcile::reconcile(rcd, &index, usable, "", apply)
                    .await
                    .map_err(|e| e.to_string())?;
                report.unreachable.extend(unreachable.iter().cloned());
                let gaps = repair::find_gaps(&index, usable, &config, "")
                    .await
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>(CheckOutcome { report, gaps })
            }
            .await;
            cache.release(connection, outcome.is_ok()).await;
            outcome
        })
    }

    /// Rebuilds the missing copies of files that have fewer than wanted.
    pub fn repair_copies(&self) -> JoinHandle<Result<repair::Report, String>> {
        let index = Arc::clone(&self.index);
        let sessions = Arc::clone(&self.sessions);
        self.runtime.spawn(async move {
            let mut cache = sessions.lock().await;
            let store =
                AccountStore::load(AccountStore::default_path().map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            if store.list().is_empty() {
                return Err("Connect a Mega account first.".to_owned());
            }
            let config = Config::load(Config::default_path().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            let connection = cache.checkout(&store).await?;
            let Connected {
                rcd,
                usable,
                unreachable,
            } = &connection;
            let result =
                repair::repair(rcd, &index, usable, &MostFreeSpace::default(), &config, "")
                    .await
                    .map_err(|e| e.to_string())
                    .map(|mut report| {
                        report.unreachable.extend(unreachable.iter().cloned());
                        report
                    });
            cache.release(connection, result.is_ok()).await;
            result
        })
    }

    /// Deep verification downloads replicas; cancellation stops its daemon.
    pub fn verify_copies(&self) -> VerificationJob {
        let index = Arc::clone(&self.index);
        let sessions = Arc::clone(&self.sessions);
        let (cancel, cancel_rx) = tokio::sync::oneshot::channel::<()>();
        let (tx, events) = unbounded_channel();
        let result = self.runtime.spawn(async move {
            let cancelled = async {
                if cancel_rx.await.is_err() {
                    std::future::pending::<()>().await;
                }
            };
            tokio::pin!(cancelled);
            let _ = tx.send("Waiting for the active operation to finish");
            let mut cache = tokio::select! {
                biased;
                () = &mut cancelled => return Ok(None),
                cache = sessions.lock() => cache,
            };
            let store =
                AccountStore::load(AccountStore::default_path().map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let _ = tx.send("Connecting accounts for verification");
            let connection = tokio::select! {
                biased;
                () = &mut cancelled => return Ok(None),
                connection = cache.checkout(&store) => connection?,
            };
            let Connected { rcd, usable, .. } = &connection;
            let _ = tx.send("Downloading and verifying indexed copies");
            let outcome = {
                let verify = bufonidae_core::verify::verify(rcd, &index, usable, "");
                tokio::pin!(verify);
                tokio::select! {
                    biased;
                    () = &mut cancelled => Ok(None),
                    report = &mut verify => report.map(Some).map_err(|e| e.to_string()),
                }
            };
            cache
                .release(connection, matches!(&outcome, Ok(Some(_))))
                .await;
            outcome
        });
        VerificationJob {
            cancel,
            events,
            result,
        }
    }

    /// Asks every account how much space it has. This signs in to each one, so
    /// the desktop requests this once after loading accounts or on explicit refresh.
    pub fn check_health(&self) -> JoinHandle<Result<Vec<AccountHealth>, String>> {
        let sessions = Arc::clone(&self.sessions);
        self.runtime.spawn(async move {
            let mut cache = sessions.lock().await;
            cache.invalidate().await;
            let store =
                AccountStore::load(AccountStore::default_path().map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            if store.list().is_empty() {
                return Ok(Vec::new());
            }
            let rcd = start_rclone().await?;
            let secrets: SharedSecrets = Arc::new(KeyringStore);
            let mut health = Vec::new();
            for (account, signed_in) in manager::connect_all(&rcd, &store, &secrets).await {
                let status = match signed_in {
                    Err(e) => {
                        HealthStatus::Unreachable(bufonidae_rclone::escape_control(&e.to_string()))
                    }
                    Ok(()) => {
                        let remote = format!("{}:", account.remote_name());
                        let ask = rcd.about(&remote);
                        match tokio::time::timeout(std::time::Duration::from_secs(30), ask).await {
                            Ok(Ok(about)) => HealthStatus::Ok {
                                total: about.total,
                                used: about.used,
                                free: about.free,
                            },
                            Ok(Err(e)) => HealthStatus::Unreachable(e.to_string()),
                            Err(_) => HealthStatus::Unreachable(
                                "it did not answer within 30 seconds".to_owned(),
                            ),
                        }
                    }
                };
                health.push(AccountHealth {
                    id: account.id,
                    status,
                });
            }
            let _ = rcd.shutdown().await;
            Ok(health)
        })
    }

    /// Signs an existing account in again (new password, or after a restore).
    pub fn reauthorize(
        &self,
        label: String,
        password: String,
        two_fa: Option<String>,
    ) -> JoinHandle<Result<(), String>> {
        let sessions = Arc::clone(&self.sessions);
        self.runtime.spawn(async move {
            let mut cache = sessions.lock().await;
            cache.invalidate().await;
            let store =
                AccountStore::load(AccountStore::default_path().map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let rcd = start_rclone().await?;
            let secrets: SharedSecrets = Arc::new(KeyringStore);
            let result =
                manager::reauthorize(&rcd, &store, &secrets, &label, &password, two_fa.as_deref())
                    .await;
            let _ = rcd.shutdown().await;
            result.map(|_| ()).map_err(|e| e.to_string())
        })
    }

    /// Forgets an account and its stored secrets. Files stay in the Mega
    /// account; the index keeps its record of them.
    pub fn remove_account(&self, label: String) -> JoinHandle<Result<(), String>> {
        let sessions = Arc::clone(&self.sessions);
        self.runtime.spawn(async move {
            let mut cache = sessions.lock().await;
            cache.invalidate().await;
            let mut store =
                AccountStore::load(AccountStore::default_path().map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let secrets: SharedSecrets = Arc::new(KeyringStore);
            manager::remove_account(&mut store, &secrets, &label)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
    }

    /// Connects a Mega account: proves the login through rclone, then stores
    /// the account and its secrets. Nothing is kept when it fails.
    pub fn add_account(&self, request: NewAccount) -> JoinHandle<Result<(), String>> {
        let sessions = Arc::clone(&self.sessions);
        self.runtime.spawn(async move {
            let mut cache = sessions.lock().await;
            cache.invalidate().await;
            let path = AccountStore::default_path().map_err(|e| e.to_string())?;
            let mut store = AccountStore::load(path).map_err(|e| e.to_string())?;
            // Cheap checks first, so a typo does not wait for a login attempt.
            store
                .prepare(&request.label, &request.email)
                .map_err(|e| e.to_string())?;

            let rcd = start_rclone().await?;
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

/// What the store dialog needs to know before it starts: how big the drop is
/// and whether the accounts can satisfy the redundancy settings.
#[derive(Debug, Clone)]
pub struct Inspection {
    pub files: usize,
    pub bytes: u64,
    /// Symlinks and unsupported names, which are never uploaded.
    pub skipped: usize,
    pub accounts: usize,
    /// One of the dropped paths is the home folder or contains it.
    pub includes_home: bool,
    pub config: Config,
}

/// How a store job ended.
pub enum StoreOutcome {
    Done(put::Report),
    /// The user cancelled. Copies already stored stay stored; an interrupted
    /// one is left `pending` in the index for `reconcile`.
    Cancelled,
}

/// A running store job: progress events, a way to cancel, then the result.
pub enum TransferEvent<T> {
    Stage(&'static str),
    Progress(T),
    Live(Option<bufonidae_rclone::rcd::TransferStats>),
}

pub struct StoreJob {
    pub events: UnboundedReceiver<TransferEvent<Progress>>,
    /// Sending on it asks the job to stop.
    pub cancel: tokio::sync::oneshot::Sender<()>,
    pub result: JoinHandle<Result<StoreOutcome, String>>,
}

pub struct VerificationJob {
    pub events: UnboundedReceiver<&'static str>,
    pub cancel: tokio::sync::oneshot::Sender<()>,
    pub result: JoinHandle<Result<Option<bufonidae_core::verify::Report>, String>>,
}

/// How a download job ended.
pub enum DownloadOutcome {
    Done(get::Report),
    /// The user cancelled. Finished files stay; rclone may leave a
    /// `.partial` temp file for the one in flight.
    Cancelled,
}

/// A running download job: progress events, a way to cancel, then the result.
pub struct DownloadJob {
    pub events: UnboundedReceiver<TransferEvent<get::Progress>>,
    pub cancel: tokio::sync::oneshot::Sender<()>,
    pub result: JoinHandle<Result<DownloadOutcome, String>>,
}

/// Starts rclone, with a message a person can act on when it is missing or
/// not a supported version.
async fn start_rclone() -> Result<Rcd, String> {
    Rcd::start(bufonidae_rclone::locate())
        .await
        .map_err(|e| match e {
            RcloneError::Spawn(io) if io.kind() == std::io::ErrorKind::NotFound => {
                "rclone was not found. Install it and make sure it is on your PATH.".to_owned()
            }
            e => e.to_string(),
        })
}

/// Poll concurrently with the transfer, never blocking its cancellation.
async fn observe_transfer_bytes<T>(
    rcd: &Rcd,
    tx: &tokio::sync::mpsc::UnboundedSender<TransferEvent<T>>,
    enabled: bool,
) {
    if !enabled {
        std::future::pending::<()>().await;
    }
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(250));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        let sample = tokio::time::timeout(std::time::Duration::from_secs(2), rcd.transfer_stats())
            .await
            .ok()
            .and_then(Result::ok);
        let _ = tx.send(TransferEvent::Live(sample));
    }
}

/// rclone with the accounts that could be defined in it.
struct Connected {
    rcd: Rcd,
    /// Accounts that signed in and may be used.
    usable: Vec<Account>,
    /// Accounts that could not be reached, with why: `(label, reason)`.
    unreachable: Vec<(String, String)>,
}

/// One lease at a time: a cancelled operation can stop its daemon without
/// affecting another transfer, and placement never reserves the same space twice.
#[derive(Default)]
struct SessionCache {
    accounts: Vec<Account>,
    connection: Option<Connected>,
}

impl SessionCache {
    async fn checkout(&mut self, store: &AccountStore) -> Result<Connected, String> {
        if let Some(mut connection) = self.connection.take() {
            if self.accounts == store.list()
                && connection.unreachable.is_empty()
                && connection.rcd.is_running().map_err(|e| e.to_string())?
            {
                return Ok(connection);
            }
            let _ = connection.rcd.shutdown().await;
        }
        let connection = connected_rcd(store).await?;
        self.accounts = store.list().to_vec();
        Ok(connection)
    }

    async fn release(&mut self, connection: Connected, reusable: bool) {
        if reusable {
            self.connection = Some(connection);
        } else {
            let _ = connection.rcd.shutdown().await;
        }
    }

    async fn invalidate(&mut self) {
        if let Some(connection) = self.connection.take() {
            let _ = connection.rcd.shutdown().await;
        }
        self.accounts.clear();
    }
}

/// Starts rclone and defines every stored account in it. An account that fails
/// to sign in is set aside and reported rather than stopping the others, so
/// that one broken account does not block files held by the healthy ones.
/// Fails only when no account could be used, stopping rclone first.
async fn connected_rcd(store: &AccountStore) -> Result<Connected, String> {
    let rcd = start_rclone().await?;
    let secrets: SharedSecrets = Arc::new(KeyringStore);
    let mut usable = Vec::new();
    let mut unreachable = Vec::new();
    for (account, result) in manager::connect_all(&rcd, store, &secrets).await {
        match result {
            Ok(()) => usable.push(account),
            Err(e) => unreachable.push((account.label, e.to_string())),
        }
    }
    if usable.is_empty() {
        let _ = rcd.shutdown().await;
        let reasons: Vec<String> = unreachable
            .iter()
            .map(|(label, why)| format!("{label}: {why}"))
            .collect();
        return Err(format!("No account could be used. {}", reasons.join("; ")));
    }
    Ok(Connected {
        rcd,
        usable,
        unreachable,
    })
}

/// A change to the redundancy settings.
pub enum RedundancyChange {
    /// Copies for every file without a folder rule.
    Default(u8),
    /// Copies for one folder (and what is under it).
    Rule {
        folder: String,
        copies: u8,
    },
    RemoveRule(String),
}

/// What a files check found.
pub struct CheckOutcome {
    pub report: reconcile::Report,
    /// Files with fewer copies than the redundancy settings ask for.
    pub gaps: Vec<repair::Gap>,
}

/// How one account is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountHealth {
    pub id: String,
    pub status: HealthStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthStatus {
    /// Signed in. Mega may leave a figure unknown.
    Ok {
        total: Option<u64>,
        used: Option<u64>,
        free: Option<u64>,
    },
    /// Could not sign in or did not answer, with why.
    Unreachable(String),
}

/// One entry of the file browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntryInfo {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    /// Links are shown but cannot be chosen: `put` never follows them.
    pub is_symlink: bool,
}

/// Folders first, then files, each group by name ignoring case.
pub fn read_dir_sorted(dir: &std::path::Path) -> Result<Vec<DirEntryInfo>, String> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let Ok(entry) = entry else { continue };
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        entries.push(DirEntryInfo {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: entry.path(),
            is_dir: meta.is_dir(),
            is_symlink: meta.is_symlink(),
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// `true` when `root` is `home` itself or a folder that contains it, i.e.
/// storing `root` would include the whole home folder.
pub fn covers(root: &std::path::Path, home: &std::path::Path) -> bool {
    home.starts_with(root)
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
    fn queued_download_can_be_cancelled_without_waiting_for_the_active_operation() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let index = runtime.block_on(Index::open_memory()).unwrap();
        let backend = Backend {
            tray: None,
            runtime,
            index: Arc::new(index),
            sessions: Arc::new(tokio::sync::Mutex::new(SessionCache::default())),
        };
        // An earlier operation owns the daemon. Cancellation must not wait for
        // it or access account credentials before acquiring its own lease.
        let active = backend
            .runtime
            .block_on(Arc::clone(&backend.sessions).lock_owned());
        let out = tempfile::tempdir().unwrap();
        let job = backend.download_selection(vec!["Docs/file".into()], out.path().to_owned());
        job.cancel.send(()).unwrap();
        let outcome = backend.runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                job.result.await.unwrap().unwrap()
            })
            .await
        });
        assert!(matches!(outcome, Ok(DownloadOutcome::Cancelled)));
        let jobs = backend.runtime.block_on(backend.index.jobs()).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].state, "cancelled");
        assert!(std::fs::read_dir(out.path()).unwrap().next().is_none());
        drop(active);
    }

    #[test]
    fn queued_verification_cancel_does_not_access_accounts_or_wait_for_the_daemon() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let index = runtime.block_on(Index::open_memory()).unwrap();
        let backend = Backend {
            tray: None,
            runtime,
            index: Arc::new(index),
            sessions: Arc::new(tokio::sync::Mutex::new(SessionCache::default())),
        };
        let active = backend
            .runtime
            .block_on(Arc::clone(&backend.sessions).lock_owned());
        let job = backend.verify_copies();
        job.cancel.send(()).unwrap();
        let outcome = backend
            .runtime
            .block_on(async {
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    job.result.await.unwrap().unwrap()
                })
                .await
            })
            .unwrap();
        assert!(outcome.is_none());
        drop(active);
    }

    #[tokio::test]
    async fn cached_daemon_keeps_remotes_and_invalidation_discards_it() {
        let rcd = match Rcd::start("rclone").await {
            Ok(rcd) => rcd,
            Err(RcloneError::Spawn(_))
                if std::env::var_os("BUFONIDAE_REQUIRE_RCLONE").is_none() =>
            {
                return;
            }
            Err(error) => panic!("could not start local rclone: {error}"),
        };
        rcd.config_create("sentinel", "local", &serde_json::Map::new())
            .await
            .unwrap();
        let mut cache = SessionCache::default();
        let connection = Connected {
            rcd,
            usable: Vec::new(),
            unreachable: Vec::new(),
        };
        cache.release(connection, true).await;
        let dir = tempfile::tempdir().unwrap();
        let store = AccountStore::load(dir.path().join("accounts.toml")).unwrap();
        let connection = cache.checkout(&store).await.unwrap();
        assert!(cache.connection.is_none());
        let remote = connection.rcd.config_get("sentinel").await.unwrap();
        assert_eq!(remote.get("type").map(String::as_str), Some("local"));
        cache.release(connection, true).await;
        cache.invalidate().await;
        assert!(cache.connection.is_none());
        assert!(cache.accounts.is_empty());
    }

    #[tokio::test]
    async fn cancelled_operation_does_not_return_a_daemon_to_the_cache() {
        let rcd = match Rcd::start("rclone").await {
            Ok(rcd) => rcd,
            Err(RcloneError::Spawn(_))
                if std::env::var_os("BUFONIDAE_REQUIRE_RCLONE").is_none() =>
            {
                return;
            }
            Err(error) => panic!("could not start local rclone: {error}"),
        };
        let mut cache = SessionCache::default();
        cache
            .release(
                Connected {
                    rcd,
                    usable: Vec::new(),
                    unreachable: Vec::new(),
                },
                false,
            )
            .await;
        assert!(cache.connection.is_none());
    }

    #[test]
    fn counts_files_that_exist_on_one_account_only() {
        let file = |path: &str, accounts: &[&str]| IndexedFile {
            path: path.to_owned(),
            size: 10,
            accounts: accounts.iter().map(|a| (*a).to_owned()).collect(),
            blake3: None,
        };
        let snapshot = Snapshot {
            accounts: Vec::new(),
            pending: 0,
            jobs: Vec::new(),
            files: vec![
                file("only-a", &["a"]),
                file("both", &["a", "b"]),
                file("only-b", &["b"]),
                file("also-only-a", &["a"]),
            ],
        };
        assert_eq!(snapshot.only_on("a"), 2);
        assert_eq!(snapshot.only_on("b"), 1);
        assert_eq!(snapshot.only_on("nobody"), 0);
        assert_eq!(snapshot.usage("a"), (3, 30), "all files with a copy on a");
    }

    #[test]
    fn covers_the_home_folder_and_its_ancestors_only() {
        use std::path::Path;
        let home = Path::new("/home/ana");
        assert!(covers(Path::new("/home/ana"), home));
        assert!(covers(Path::new("/home"), home));
        assert!(covers(Path::new("/"), home));
        assert!(!covers(Path::new("/home/ana/Pictures"), home));
        assert!(!covers(Path::new("/home/anabel"), home));
        assert!(!covers(Path::new("/tmp"), home));
    }

    #[test]
    fn lists_folders_first_then_files_ignoring_case() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("zeta")).unwrap();
        std::fs::create_dir(dir.path().join("Alpha")).unwrap();
        std::fs::write(dir.path().join("b.txt"), b"x").unwrap();
        std::fs::write(dir.path().join("A.txt"), b"x").unwrap();
        std::os::unix::fs::symlink(dir.path().join("b.txt"), dir.path().join("link")).unwrap();

        let entries = read_dir_sorted(dir.path()).unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "zeta", "A.txt", "b.txt", "link"]);
        assert!(entries.last().unwrap().is_symlink);
        assert!(read_dir_sorted(&dir.path().join("missing")).is_err());
    }

    #[test]
    fn suggests_a_valid_label_from_the_email() {
        assert_eq!(suggest_label("ana.silva@example.com"), "ana.silva");
        assert_eq!(suggest_label("a b+c@example.com"), "abc");
        assert_eq!(suggest_label("@example.com"), "account");
        assert_eq!(suggest_label(""), "account");
        assert_eq!(suggest_label(&format!("{}@x.y", "z".repeat(50))).len(), 32);
    }
}
