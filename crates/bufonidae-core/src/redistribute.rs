//! Reviewed redistribution plans. Every new copy is read back before retiring
//! an old one. Remote mutations outside this process must be paused during sync.
use crate::{
    account::Account,
    config::Config,
    index::{self, Index, IndexedFile},
    put::{self, RemoteFs},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Index(#[from] index::Error),
    #[error(transparent)]
    Remote(#[from] put::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Balance,
    Repair,
    Mirror(u8),
    Evacuate(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Options {
    pub mode: Mode,
    /// Actual rclone caps, in bytes/second. Both must be nonzero.
    pub download_rate: u64,
    pub upload_rate: u64,
    pub headroom_percent: u8,
    pub latency_ms: u32,
    /// Explicit user estimate; no provider reset window is inferred.
    pub quota_wait_seconds: u64,
    /// Preview download ceiling, including verification reads; retries can exceed it.
    pub download_budget: Option<u64>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            mode: Mode::Balance,
            download_rate: 10_000_000,
            upload_rate: 5_000_000,
            headroom_percent: 5,
            latency_ms: 100,
            quota_wait_seconds: 0,
            download_budget: None,
        }
    }
}
impl Options {
    pub fn validate(&self) -> Result<(), Error> {
        if self.download_rate == 0
            || self.upload_rate == 0
            || self.headroom_percent > 50
            || self.latency_ms > 60_000
            || self.quota_wait_seconds > 365 * 86400
        {
            return Err(Error::Invalid("Rates must be positive; headroom must be 0–50%, latency at most 60 seconds and quota wait at most one year.".into()));
        }
        if matches!(self.mode, Mode::Mirror(n) if !(2..=8).contains(&n)) {
            return Err(Error::Invalid("Mirror needs 2–8 distinct accounts.".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quota {
    pub id: String,
    pub label: String,
    pub total: u64,
    pub used: u64,
}
impl Quota {
    fn free(&self) -> u64 {
        self.total.saturating_sub(self.used)
    }
    fn reserve(&self, options: &Options) -> u64 {
        (u128::from(self.total) * u128::from(options.headroom_percent) / 100) as u64
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub path: String,
    pub size: u64,
    pub hash: Option<String>,
    pub sources: Vec<String>,
    pub add: Vec<String>,
    pub remove: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub options: Options,
    pub quotas: Vec<Quota>,
    pub projected: Vec<Quota>,
    pub changes: Vec<Change>,
    pub download_bytes: u64,
    pub upload_bytes: u64,
    pub temporary_bytes: u64,
    pub latency_calls: u64,
    pub warnings: Vec<String>,
}
impl Plan {
    /// Serial legs, not min(down, up): staged copies cannot overlap these legs.
    pub fn seconds(&self) -> f64 {
        self.download_bytes as f64 / self.options.download_rate as f64
            + self.upload_bytes as f64 / self.options.upload_rate as f64
            + self.latency_calls as f64 * f64::from(self.options.latency_ms) / 1000.0
            + self.options.quota_wait_seconds as f64
    }
}
fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}
fn plus(a: u64, b: u64) -> Result<u64, Error> {
    a.checked_add(b)
        .ok_or_else(|| invalid("Plan size overflow."))
}
fn times(a: u64, b: usize) -> Result<u64, Error> {
    a.checked_mul(b as u64)
        .ok_or_else(|| invalid("Plan size overflow."))
}

/// Pure, deterministic planner. Unknown quota/unreachable accounts must be
/// resolved by the caller; it never silently treats them as empty capacity.
pub fn plan(
    files: &[IndexedFile],
    quotas: &[Quota],
    config: &Config,
    options: Options,
) -> Result<Plan, Error> {
    options.validate()?;
    let mut quotas = quotas.to_vec();
    quotas.sort_by(|a, b| a.id.cmp(&b.id));
    let ids: BTreeSet<_> = quotas.iter().map(|q| q.id.as_str()).collect();
    if quotas.is_empty() || ids.len() != quotas.len() || quotas.iter().any(|q| q.total == 0) {
        return Err(invalid(
            "All connected accounts need distinct IDs and known nonzero quotas.",
        ));
    }
    if let Mode::Evacuate(id) = &options.mode
        && !ids.contains(id.as_str())
    {
        return Err(invalid("The evacuation account is no longer connected."));
    }
    let mut ordered = files.to_vec();
    ordered.sort_by(|a, b| b.size.cmp(&a.size).then(a.path.cmp(&b.path)));
    let mut paths = BTreeSet::new();
    for file in &ordered {
        let holders: BTreeSet<_> = file.accounts.iter().collect();
        if !crate::paths::is_safe_stored_path(&file.path)
            || !paths.insert(&file.path)
            || holders.is_empty()
            || holders.len() != file.accounts.len()
            || file.accounts.iter().any(|id| !ids.contains(id.as_str()))
        {
            return Err(invalid(format!(
                "{}: unsafe/stale placements. Run Check and connect every indexed holder before sync.",
                file.path
            )));
        }
    }
    let already_balanced = matches!(options.mode, Mode::Balance)
        && quotas.first().is_some_and(|first| {
            quotas.iter().all(|q| {
                u128::from(q.used) * u128::from(first.total)
                    == u128::from(first.used) * u128::from(q.total)
            })
        })
        && ordered
            .iter()
            .all(|file| file.accounts.len() >= usize::from(config.copies_for(&file.path)));
    let preserve_layout = matches!(options.mode, Mode::Repair) || already_balanced;
    let mut projected = quotas.clone();
    // Always validate indexed usage, even when keeping existing placements.
    for q in &mut projected {
        let indexed = ordered
            .iter()
            .filter(|f| f.accounts.contains(&q.id))
            .try_fold(0, |sum, f| plus(sum, f.size))?;
        if indexed > q.used {
            return Err(invalid(format!(
                "{}: indexed bytes exceed reported usage. Refresh storage and run Check.",
                q.label
            )));
        }
        if !preserve_layout {
            q.used -= indexed;
        }
    }
    let mut changes = Vec::new();
    for file in &ordered {
        let floor = usize::from(config.copies_for(&file.path));
        let wanted = match options.mode {
            Mode::Mirror(n) => floor.max(usize::from(n)).max(file.accounts.len()),
            _ => floor.max(file.accounts.len()),
        };
        let mut desired = if preserve_layout {
            file.accounts.clone()
        } else {
            Vec::new()
        };
        while desired.len() < wanted {
            let target = projected
                .iter()
                .enumerate()
                .filter(|(_, q)| {
                    !desired.contains(&q.id)
                        && !matches!(&options.mode, Mode::Evacuate(id) if id == &q.id)
                        && q.free().saturating_sub(q.reserve(&options)) >= file.size
                })
                .min_by(|(_, a), (_, b)| {
                    // Compare fractions exactly. Prefer existing placements on ties.
                    (u128::from(a.used + file.size) * u128::from(b.total))
                        .cmp(&(u128::from(b.used + file.size) * u128::from(a.total)))
                        .then_with(|| {
                            file.accounts
                                .contains(&b.id)
                                .cmp(&file.accounts.contains(&a.id))
                        })
                        .then(a.id.cmp(&b.id))
                })
                .map(|(i, _)| i);
            let Some(target) = target else {
                return Err(invalid(format!(
                    "{}: cannot place all {wanted} copies with the selected reserve and account exclusions.",
                    file.path
                )));
            };
            projected[target].used = plus(projected[target].used, file.size)?;
            desired.push(projected[target].id.clone());
        }
        let add: Vec<_> = desired
            .iter()
            .filter(|id| !file.accounts.contains(id))
            .cloned()
            .collect();
        let remove: Vec<_> = file
            .accounts
            .iter()
            .filter(|id| !desired.contains(id))
            .cloned()
            .collect();
        if !add.is_empty() || !remove.is_empty() {
            changes.push(Change {
                path: file.path.clone(),
                size: file.size,
                hash: file.blake3.clone(),
                sources: file.accounts.clone(),
                add,
                remove,
            });
        }
    }
    // Find a serial order that fits copy-before-delete peak capacity, not only
    // final placement. Cycles requiring unavailable scratch space are refused.
    let mut free: BTreeMap<_, _> = quotas.iter().map(|q| (q.id.clone(), q.free())).collect();
    let reserves: BTreeMap<_, _> = quotas
        .iter()
        .map(|q| (q.id.clone(), q.reserve(&options)))
        .collect();
    let mut scheduled = Vec::new();
    while !changes.is_empty() {
        let next = changes.iter().position(|c| {
            c.add
                .iter()
                .all(|id| free[id].saturating_sub(reserves[id]) >= c.size)
        });
        let Some(i) = next else {
            return Err(invalid(
                "Copy-before-delete needs more free remote workspace. Lower the reserve, connect another account, or choose Repair. No files were moved.",
            ));
        };
        let change = changes.remove(i);
        for id in &change.add {
            *free
                .get_mut(id)
                .ok_or_else(|| invalid("Unknown destination"))? -= change.size;
        }
        for id in &change.remove {
            let value = free.get_mut(id).ok_or_else(|| invalid("Unknown source"))?;
            *value = plus(*value, change.size)?;
        }
        scheduled.push(change);
    }
    let mut result = Plan { options, quotas, projected, changes: scheduled, download_bytes: 0, upload_bytes: 0, temporary_bytes: 0, latency_calls: 0, warnings: vec!["Provider transfer quotas/reset times are not reported by the storage API. Known waiting time must be entered separately; actual throttling/retries can extend the estimate.".into(), "Pause external edits to these remotes while sync runs. Only indexed files are managed; parity and block striping are not supported.".into()] };
    if !result.changes.is_empty() {
        result.latency_calls = plus(times(result.quotas.len() as u64, 3)?, 3)?;
    }
    for c in &result.changes {
        // Initial source + readback of every upload + recheck of every removal.
        result.download_bytes = plus(
            result.download_bytes,
            times(c.size, 1 + c.add.len() + c.remove.len())?,
        )?;
        result.upload_bytes = plus(result.upload_bytes, times(c.size, c.add.len())?)?;
        result.temporary_bytes = result.temporary_bytes.max(times(c.size, 2)?);
        result.latency_calls = plus(
            result.latency_calls,
            (1 + c.add.len() * 2 + c.remove.len() * 2) as u64,
        )?;
        if c.hash.is_none() {
            result.warnings.push(format!("{} has no trusted baseline hash: equality to the downloaded source will be verified, not historical integrity.", c.path));
        }
    }
    if result
        .options
        .download_budget
        .is_some_and(|budget| result.download_bytes > budget)
    {
        return Err(invalid(format!(
            "Plan needs {} download bytes, exceeding the entered session budget (verification included).",
            result.download_bytes
        )));
    }
    Ok(result)
}

/// Extra operations required by redistribution, leaving ordinary RemoteFs
/// implementations and existing transfers unchanged.
pub trait SyncRemote: RemoteFs {
    fn upload_new(
        &self,
        account: &Account,
        parent: &Path,
        name: &str,
        remote: &str,
    ) -> impl std::future::Future<Output = Result<(), put::Error>>;
    fn remove(
        &self,
        account: &Account,
        remote: &str,
    ) -> impl std::future::Future<Output = Result<(), put::Error>>;
}
impl SyncRemote for bufonidae_rclone::rcd::Rcd {
    async fn upload_new(
        &self,
        account: &Account,
        parent: &Path,
        name: &str,
        remote: &str,
    ) -> Result<(), put::Error> {
        let parent = parent
            .to_str()
            .filter(|_| parent.is_absolute())
            .ok_or_else(|| put::Error::Remote("Invalid staging folder".into()))?;
        let _: serde_json::Value = self.call("operations/copyfile", &serde_json::json!({"srcFs":parent,"srcRemote":name,"dstFs":format!("{}:",account.remote_name()),"dstRemote":remote,"_config":{"IgnoreExisting":true}})).await.map_err(|e|put::Error::Remote(e.to_string()))?;
        Ok(())
    }
    async fn remove(&self, account: &Account, remote: &str) -> Result<(), put::Error> {
        let _: serde_json::Value = self
            .call(
                "operations/deletefile",
                &serde_json::json!({"fs":format!("{}:",account.remote_name()),"remote":remote}),
            )
            .await
            .map_err(|e| put::Error::Remote(e.to_string()))?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Progress {
    pub completed: usize,
    pub total: usize,
    pub stage: &'static str,
    pub path: String,
}

/// A completed file is durable. Cancellation must be honored *between* files:
/// dropping this future during delete/index commit could leave stale placements.
#[allow(clippy::too_many_arguments)]
pub async fn execute<R: SyncRemote>(
    remote: &R,
    index: &Index,
    accounts: &[Account],
    config: &Config,
    reviewed: &Plan,
    staging: &Path,
    mut cancelled: impl FnMut() -> bool,
    mut progress: impl FnMut(Progress),
) -> Result<usize, Error> {
    if !staging.is_absolute() || !staging.is_dir() {
        return Err(invalid("Choose an existing absolute staging folder."));
    }
    let files = index.list("").await?;
    let fresh = plan(&files, &reviewed.quotas, config, reviewed.options.clone())?;
    if fresh.changes != reviewed.changes {
        return Err(invalid(
            "The index or redundancy policy changed. Generate a fresh preview.",
        ));
    }
    let by_id: BTreeMap<_, _> = accounts.iter().map(|a| (a.id.as_str(), a)).collect();
    let mut listings = BTreeMap::new();
    for quota in &reviewed.quotas {
        let account = by_id
            .get(quota.id.as_str())
            .ok_or_else(|| invalid("An account was disconnected. Preview again."))?;
        if remote
            .free_space(account)
            .await?
            .is_none_or(|free| free < quota.free())
        {
            return Err(invalid("Remote free space changed. Refresh the preview."));
        }
        listings.insert(quota.id.as_str(), remote.list_files(account, "").await?);
    }
    for c in &reviewed.changes {
        for id in &c.sources {
            let listing = &listings[id.as_str()];
            if listing.duplicates.contains(&c.path) || listing.files.get(&c.path) != Some(&c.size) {
                return Err(invalid(format!(
                    "{}: source changed or has duplicate names. Run Check.",
                    c.path
                )));
            }
        }
        for id in &c.add {
            let listing = &listings[id.as_str()];
            if listing.duplicates.contains(&c.path) || listing.files.contains_key(&c.path) {
                return Err(invalid(format!(
                    "{} already exists at a destination. Reconcile it before sync; nothing will be overwritten.",
                    c.path
                )));
            }
        }
    }
    let mut completed = 0;
    for c in &reviewed.changes {
        if cancelled() {
            break;
        }
        let emit = |stage| Progress {
            completed,
            total: reviewed.changes.len(),
            stage,
            path: c.path.clone(),
        };
        progress(emit("Downloading source"));
        let temp = tempfile::tempdir_in(staging)?;
        let local = temp.path().join("source");
        let mut fetched = false;
        let mut last = String::new();
        for id in &c.sources {
            match remote
                .download(by_id[id.as_str()], &c.path, temp.path(), "source")
                .await
            {
                Ok(()) => match crate::get::verify(&local, c.size, c.hash.as_deref()).await {
                    Ok(()) => {
                        fetched = true;
                        break;
                    }
                    Err(e) => last = e,
                },
                Err(e) => last = e.to_string(),
            }
            if local.exists() {
                std::fs::remove_file(&local)?;
            }
        }
        if !fetched {
            return Err(invalid(format!("{}: no readable source: {last}", c.path)));
        }
        let hash_path = local.clone();
        let hash = tokio::task::spawn_blocking(move || crate::hashing::hash_file(&hash_path))
            .await
            .map_err(|e| invalid(e.to_string()))??;
        for id in &c.add {
            progress(emit("Uploading new copy"));
            index
                .mark_pending_hashed(&c.path, c.size, id, c.hash.as_deref())
                .await?;
            remote
                .upload_new(by_id[id.as_str()], temp.path(), "source", &c.path)
                .await?;
            progress(emit("Reading back and verifying destination"));
            remote
                .download(by_id[id.as_str()], &c.path, temp.path(), "verify")
                .await?;
            crate::get::verify(&temp.path().join("verify"), c.size, Some(&hash))
                .await
                .map_err(invalid)?;
            std::fs::remove_file(temp.path().join("verify"))?;
            index.mark_done(&c.path, id).await?;
        }
        // All new placements have been confirmed before any source is retired.
        for id in &c.remove {
            progress(emit("Rechecking original before removal"));
            remote
                .download(by_id[id.as_str()], &c.path, temp.path(), "verify")
                .await?;
            crate::get::verify(&temp.path().join("verify"), c.size, Some(&hash))
                .await
                .map_err(invalid)?;
            std::fs::remove_file(temp.path().join("verify"))?;
            progress(emit("Retiring verified old copy"));
            remote.remove(by_id[id.as_str()], &c.path).await?;
            index.remove_placement(&c.path, id).await?;
        }
        completed += 1;
        progress(Progress {
            completed,
            total: reviewed.changes.len(),
            stage: "File synchronized",
            path: c.path.clone(),
        });
    }
    Ok(completed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    fn quotas() -> Vec<Quota> {
        vec![
            Quota {
                id: "a".into(),
                label: "A".into(),
                total: 100,
                used: 90,
            },
            Quota {
                id: "b".into(),
                label: "B".into(),
                total: 100,
                used: 0,
            },
        ]
    }
    fn file() -> IndexedFile {
        IndexedFile {
            path: "folder/file".into(),
            size: 60,
            accounts: vec!["a".into()],
            blake3: Some(blake3::hash(&[42; 60]).to_hex().to_string()),
        }
    }
    #[test]
    fn balance_accounts_for_every_verification_leg_and_peak_staging() {
        let p = plan(
            &[file()],
            &quotas(),
            &Config::default(),
            Options {
                download_rate: 10,
                upload_rate: 5,
                latency_ms: 0,
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(p.changes[0].add, vec!["b"]);
        assert_eq!(p.changes[0].remove, vec!["a"]);
        assert_eq!(
            (p.download_bytes, p.upload_bytes, p.temporary_bytes),
            (180, 60, 120)
        );
        assert_eq!(p.seconds(), 30.0);
        assert_eq!((p.projected[0].used, p.projected[1].used), (30, 60));
        let p2 = plan(
            &[file()],
            &quotas(),
            &Config::default(),
            Options {
                mode: Mode::Repair,
                ..Options::default()
            },
        )
        .unwrap();
        assert!(p2.changes.is_empty());
    }
    #[test]
    fn mirror_respects_policy_floor_and_preview_budget() {
        let p = plan(
            &[file()],
            &quotas(),
            &Config::default(),
            Options {
                mode: Mode::Mirror(2),
                ..Options::default()
            },
        )
        .unwrap();
        assert!(p.changes[0].remove.is_empty());
        assert_eq!(p.download_bytes, 120);
        let options = Options {
            download_budget: Some(119),
            ..p.options.clone()
        };
        assert!(plan(&[file()], &quotas(), &Config::default(), options).is_err());
        let config_dir = tempfile::tempdir().unwrap();
        let mut config = Config::load(config_dir.path().join("config.toml")).unwrap();
        config.set(None, 3).unwrap();
        assert!(plan(&[file()], &quotas(), &config, p.options).is_err());
    }
    #[test]
    fn refuses_stale_unsafe_and_insufficient_workspace_plans() {
        let mut f = file();
        f.path = "../escape".into();
        assert!(plan(&[f], &quotas(), &Config::default(), Options::default()).is_err());
        let mut q = quotas();
        q[1].used = 70;
        assert!(
            plan(
                &[file()],
                &q,
                &Config::default(),
                Options {
                    mode: Mode::Evacuate("a".into()),
                    ..Options::default()
                }
            )
            .is_err()
        );
        let mut q = quotas();
        q[0].used = 0;
        assert!(plan(&[file()], &q, &Config::default(), Options::default()).is_err());
        assert!(
            plan(
                &[file()],
                &quotas(),
                &Config::default(),
                Options {
                    download_rate: 0,
                    ..Options::default()
                }
            )
            .is_err()
        );
    }

    struct Fake {
        root: tempfile::TempDir,
        calls: Mutex<Vec<String>>,
        corrupt: bool,
        change_source: bool,
    }
    impl Fake {
        fn new(corrupt: bool) -> Self {
            let root = tempfile::tempdir().unwrap();
            for id in ["a", "b"] {
                std::fs::create_dir_all(root.path().join(id).join("folder")).unwrap();
            }
            std::fs::write(root.path().join("a/folder/file"), [42; 60]).unwrap();
            Self {
                root,
                calls: Mutex::new(vec![]),
                corrupt,
                change_source: false,
            }
        }
        fn path(&self, a: &Account, p: &str) -> std::path::PathBuf {
            self.root.path().join(&a.id).join(p)
        }
    }
    impl RemoteFs for Fake {
        async fn free_space(&self, a: &Account) -> Result<Option<u64>, put::Error> {
            Ok(Some(if a.id == "a" { 10 } else { 100 }))
        }
        async fn list_files(&self, a: &Account, _: &str) -> Result<put::Listing, put::Error> {
            let p = self.path(a, "folder/file");
            Ok(put::Listing::from_entries(if p.exists() {
                vec![("folder/file".into(), 60)]
            } else {
                vec![]
            }))
        }
        async fn download(
            &self,
            a: &Account,
            p: &str,
            d: &Path,
            n: &str,
        ) -> Result<(), put::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("download:{}", a.id));
            std::fs::copy(self.path(a, p), d.join(n))
                .map_err(|e| put::Error::Remote(e.to_string()))?;
            Ok(())
        }
        async fn upload(&self, _: &Account, _: &Path, _: &str, _: &str) -> Result<(), put::Error> {
            panic!("use upload_new")
        }
    }
    impl SyncRemote for Fake {
        async fn upload_new(
            &self,
            a: &Account,
            p: &Path,
            n: &str,
            r: &str,
        ) -> Result<(), put::Error> {
            self.calls.lock().unwrap().push(format!("upload:{}", a.id));
            if self.corrupt {
                std::fs::write(self.path(a, r), [7; 60]).unwrap();
            } else {
                std::fs::copy(p.join(n), self.path(a, r)).unwrap();
            }
            if self.change_source {
                std::fs::write(self.root.path().join("a/folder/file"), [99; 60]).unwrap();
            }
            Ok(())
        }
        async fn remove(&self, a: &Account, p: &str) -> Result<(), put::Error> {
            self.calls.lock().unwrap().push(format!("remove:{}", a.id));
            std::fs::remove_file(self.path(a, p)).unwrap();
            Ok(())
        }
    }
    fn accounts() -> Vec<Account> {
        ["a", "b"]
            .into_iter()
            .map(|id| Account {
                id: id.into(),
                label: id.into(),
                email: format!("{id}@example.test"),
            })
            .collect()
    }
    async fn seed(index: &Index) {
        let f = file();
        index
            .mark_pending_hashed(&f.path, f.size, "a", f.blake3.as_deref())
            .await
            .unwrap();
        index.mark_done(&f.path, "a").await.unwrap();
    }
    #[tokio::test]
    async fn execution_verifies_both_ends_before_retiring_source() {
        let remote = Fake::new(false);
        let index = Index::open_memory().await.unwrap();
        seed(&index).await;
        let p = plan(&[file()], &quotas(), &Config::default(), Options::default()).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            execute(
                &remote,
                &index,
                &accounts(),
                &Config::default(),
                &p,
                tmp.path(),
                || false,
                |_| {}
            )
            .await
            .unwrap(),
            1
        );
        assert_eq!(
            *remote.calls.lock().unwrap(),
            vec![
                "download:a",
                "upload:b",
                "download:b",
                "download:a",
                "remove:a"
            ]
        );
        assert_eq!(index.list("").await.unwrap()[0].accounts, vec!["b"]);
        assert!(!remote.root.path().join("a/folder/file").exists());
        assert!(std::fs::read_dir(tmp.path()).unwrap().next().is_none());
    }
    #[tokio::test]
    async fn corrupt_upload_keeps_original_and_unconfirmed_target() {
        let remote = Fake::new(true);
        let index = Index::open_memory().await.unwrap();
        seed(&index).await;
        let p = plan(&[file()], &quotas(), &Config::default(), Options::default()).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        assert!(
            execute(
                &remote,
                &index,
                &accounts(),
                &Config::default(),
                &p,
                tmp.path(),
                || false,
                |_| {}
            )
            .await
            .is_err()
        );
        assert!(remote.root.path().join("a/folder/file").exists());
        assert_eq!(index.list("").await.unwrap()[0].accounts, vec!["a"]);
        assert_eq!(index.pending().await.unwrap().len(), 1);
        assert!(
            !remote
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|c| c.starts_with("remove:"))
        );
    }
    #[tokio::test]
    async fn changed_original_is_never_deleted_after_upload() {
        let mut remote = Fake::new(false);
        remote.change_source = true;
        let index = Index::open_memory().await.unwrap();
        seed(&index).await;
        let p = plan(&[file()], &quotas(), &Config::default(), Options::default()).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        assert!(
            execute(
                &remote,
                &index,
                &accounts(),
                &Config::default(),
                &p,
                tmp.path(),
                || false,
                |_| {}
            )
            .await
            .is_err()
        );
        assert_eq!(
            std::fs::read(remote.root.path().join("a/folder/file")).unwrap(),
            vec![99; 60]
        );
        assert!(
            index.list("").await.unwrap()[0]
                .accounts
                .contains(&"b".to_owned())
        );
        assert!(
            !remote
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|c| c.starts_with("remove:"))
        );
    }
    #[test]
    fn already_balanced_accounts_do_not_shuffle_files() {
        let mut q = quotas();
        q[1].used = q[0].used;
        let p = plan(&[file()], &q, &Config::default(), Options::default()).unwrap();
        assert!(p.changes.is_empty());
        assert_eq!(p.download_bytes, 0);
    }
    #[tokio::test]
    async fn real_rclone_executes_round_trip_and_commits_before_removal() {
        use bufonidae_rclone::{Error as RcloneError, rcd::Rcd};
        let rcd = match Rcd::start("rclone").await {
            Ok(rcd) => rcd,
            Err(RcloneError::Spawn(_))
                if std::env::var_os("BUFONIDAE_REQUIRE_RCLONE").is_none() =>
            {
                return;
            }
            Err(error) => panic!("could not start rclone: {error}"),
        };
        let root = tempfile::tempdir().unwrap();
        let accounts = accounts();
        for account in &accounts {
            let path = root.path().join(&account.id);
            std::fs::create_dir_all(path.join("folder")).unwrap();
            let parameters = serde_json::json!({"remote": path})
                .as_object()
                .unwrap()
                .clone();
            rcd.config_create(&account.remote_name(), "alias", &parameters)
                .await
                .unwrap();
        }
        std::fs::write(root.path().join("a/folder/file"), [42; 60]).unwrap();
        let index = Index::open_memory().await.unwrap();
        seed(&index).await;
        let p = plan(&[file()], &quotas(), &Config::default(), Options::default()).unwrap();
        let staging = tempfile::tempdir().unwrap();
        assert_eq!(
            execute(
                &rcd,
                &index,
                &accounts,
                &Config::default(),
                &p,
                staging.path(),
                || false,
                |_| {}
            )
            .await
            .unwrap(),
            1
        );
        assert_eq!(
            std::fs::read(root.path().join("b/folder/file")).unwrap(),
            vec![42; 60]
        );
        assert!(!root.path().join("a/folder/file").exists());
        assert_eq!(index.list("").await.unwrap()[0].accounts, vec!["b"]);
        assert!(std::fs::read_dir(staging.path()).unwrap().next().is_none());
        rcd.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn cancellation_and_existing_destination_never_remove_or_overwrite() {
        let remote = Fake::new(false);
        let index = Index::open_memory().await.unwrap();
        seed(&index).await;
        let p = plan(&[file()], &quotas(), &Config::default(), Options::default()).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            execute(
                &remote,
                &index,
                &accounts(),
                &Config::default(),
                &p,
                tmp.path(),
                || true,
                |_| {}
            )
            .await
            .unwrap(),
            0
        );
        assert!(remote.calls.lock().unwrap().is_empty());
        std::fs::write(remote.root.path().join("b/folder/file"), [99; 60]).unwrap();
        assert!(
            execute(
                &remote,
                &index,
                &accounts(),
                &Config::default(),
                &p,
                tmp.path(),
                || false,
                |_| {}
            )
            .await
            .is_err()
        );
        assert_eq!(
            std::fs::read(remote.root.path().join("b/folder/file")).unwrap(),
            vec![99; 60]
        );
    }
}
