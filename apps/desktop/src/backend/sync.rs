//! Preview and execution share the transfer lease; limits never leak to other jobs.
use super::*;
use bufonidae_core::redistribute::{self, Options, Plan, Quota};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone)]
pub struct SyncPreview {
    pub plan: Plan,
    pub staging: PathBuf,
    pub local_free: u64,
}
pub struct SyncJob {
    pub cancel: Arc<AtomicBool>,
    pub events: UnboundedReceiver<redistribute::Progress>,
    pub live: UnboundedReceiver<Option<bufonidae_rclone::rcd::TransferStats>>,
    pub result: JoinHandle<Result<SyncResult, String>>,
}
pub struct SyncResult {
    pub completed: usize,
    pub planned: usize,
    pub cancelled: bool,
}
const LOCAL_RESERVE: u64 = 64 * 1024 * 1024;

async fn staging_free(rcd: &Rcd, staging: &std::path::Path) -> Result<u64, String> {
    if !staging.is_absolute() || !staging.is_dir() {
        return Err("Choose an existing absolute staging folder.".into());
    }
    let output: serde_json::Value = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        rcd.call("core/du", &serde_json::json!({"dir": staging})),
    )
    .await
    .map_err(|_| "Local disk space check timed out".to_owned())?
    .map_err(|e| e.to_string())?;
    output
        .get("info")
        .and_then(|info| info.get("Available"))
        .and_then(|n| n.as_u64())
        .ok_or_else(|| "The staging filesystem did not report available space.".into())
}
async fn quotas(connection: &Connected) -> Result<Vec<Quota>, String> {
    if !connection.unreachable.is_empty() {
        return Err("Every connected account must be reachable before redistribution. Check Accounts and retry.".into());
    }
    let mut quotas = Vec::new();
    for account in &connection.usable {
        let about = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            connection.rcd.about(&format!("{}:", account.remote_name())),
        )
        .await
        .map_err(|_| format!("{} did not report quota within 30 seconds", account.label))?
        .map_err(|e| e.to_string())?;
        let total = about
            .total
            .filter(|&t| t > 0)
            .ok_or_else(|| format!("{} does not report total space", account.label))?;
        let used = about
            .used
            .or_else(|| about.free.map(|f| total.saturating_sub(f)))
            .ok_or_else(|| format!("{} does not report used space", account.label))?;
        quotas.push(Quota {
            id: account.id.clone(),
            label: account.label.clone(),
            total,
            used,
        });
    }
    Ok(quotas)
}
async fn require_clean(index: &Index) -> Result<(), String> {
    if !index.pending().await.map_err(|e| e.to_string())?.is_empty()
        || !index.lost("").await.map_err(|e| e.to_string())?.is_empty()
    {
        return Err("Resolve pending/lost placements in Check before redistribution.".into());
    }
    Ok(())
}
impl Backend {
    pub fn preview_sync(
        &self,
        options: Options,
        staging: PathBuf,
    ) -> JoinHandle<Result<SyncPreview, String>> {
        let sessions = Arc::clone(&self.sessions);
        let index = Arc::clone(&self.index);
        self.runtime.spawn(async move {
            options.validate().map_err(|e|e.to_string())?;
            require_clean(&index).await?;
            let mut cache=sessions.lock().await;
            let store=AccountStore::load(AccountStore::default_path().map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            let config=Config::load(Config::default_path().map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            let connection=cache.checkout(&store).await?;
            let result=async {
                let quotas=quotas(&connection).await?;
                let files=index.list("").await.map_err(|e|e.to_string())?;
                let plan=redistribute::plan(&files,&quotas,&config,options).map_err(|e|e.to_string())?;
                let local_free=staging_free(&connection.rcd,&staging).await?;
                if local_free.saturating_sub(LOCAL_RESERVE)<plan.temporary_bytes { return Err("Not enough local space for two staging files plus the 64 MiB reserve. Choose another staging folder.".into()); }
                Ok(SyncPreview {plan,staging,local_free})
            }.await;
            cache.release(connection,result.is_ok()).await;
            result
        })
    }
    pub fn run_sync(&self, preview: SyncPreview) -> SyncJob {
        let index = Arc::clone(&self.index);
        let sessions = Arc::clone(&self.sessions);
        let cancel = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&cancel);
        let (tx, events) = unbounded_channel();
        let (live_tx, live) = unbounded_channel();
        let result=self.runtime.spawn(async move {
            let planned=preview.plan.changes.len();
            let payload=serde_json::json!({"plan":preview.plan,"staging":preview.staging}).to_string();
            let job_id=index.enqueue_job("sync",&payload).await.map_err(|e|e.to_string())?;
            let emit=|stage| {let _=tx.send(redistribute::Progress {completed:0,total:planned,stage,path:String::new()});};
            let wait_cancel=async { while !stopping.load(Ordering::Relaxed) {tokio::time::sleep(std::time::Duration::from_millis(100)).await;} };
            tokio::pin!(wait_cancel);
            let outcome=async {
                emit("Waiting for the active operation");
                let mut cache=tokio::select! { biased; ()=&mut wait_cancel=>return Ok(SyncResult {completed:0,planned,cancelled:true}), cache=sessions.lock()=>cache };
                require_clean(&index).await?;
                index.set_job_state(&job_id,"running",None).await.map_err(|e|e.to_string())?;
                let store=AccountStore::load(AccountStore::default_path().map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
                let config=Config::load(Config::default_path().map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
                emit("Connecting and revalidating the reviewed plan");
                let connection=tokio::select! {biased; ()=&mut wait_cancel=>return Ok(SyncResult {completed:0,planned,cancelled:true}), c=cache.checkout(&store)=>c?};
                let run=async {
                    if stopping.load(Ordering::Relaxed) {return Ok(SyncResult {completed:0,planned,cancelled:true});}
                    let fresh_quotas=quotas(&connection).await?;
                    let fresh_files=index.list("").await.map_err(|e|e.to_string())?;
                    let fresh=redistribute::plan(&fresh_files,&fresh_quotas,&config,preview.plan.options.clone()).map_err(|e|e.to_string())?;
                    if fresh.changes!=preview.plan.changes {return Err("Distribution changed since preview. Generate and review a new preview.".into());}
                    if staging_free(&connection.rcd,&preview.staging).await?.saturating_sub(LOCAL_RESERVE)<fresh.temporary_bytes {return Err("Local staging space changed. Generate a fresh preview.".into());}
                    if fresh.options.quota_wait_seconds>0 {
                        emit("Waiting for the configured quota reset delay");
                        tokio::select! {biased; ()=&mut wait_cancel=>return Ok(SyncResult {completed:0,planned,cancelled:true}), ()=tokio::time::sleep(std::time::Duration::from_secs(fresh.options.quota_wait_seconds))=>{}}
                    }
                    let rate=format!("{}B:{}B",fresh.options.upload_rate,fresh.options.download_rate);
                    let _:serde_json::Value=connection.rcd.call("core/bwlimit",&serde_json::json!({"rate":rate})).await.map_err(|e|e.to_string())?;
                    let _=connection.rcd.reset_transfer_stats().await;
                    let execute=redistribute::execute(&connection.rcd,&index,&connection.usable,&config,&fresh,&preview.staging,
                        ||stopping.load(Ordering::Relaxed),|progress|{let _=tx.send(progress);});
                    tokio::pin!(execute);
                    let completed=loop {
                        tokio::select! {
                            result=&mut execute=>break result.map_err(|e|e.to_string())?,
                            ()=tokio::time::sleep(std::time::Duration::from_millis(250))=>{
                                let stats=tokio::time::timeout(std::time::Duration::from_secs(2),connection.rcd.transfer_stats()).await.ok().and_then(Result::ok);
                                let _=live_tx.send(stats);
                            }
                        }
                    };
                    Ok(SyncResult {completed,planned,cancelled:completed<planned})
                }.await;
                // Discard the daemon even on success: caps cannot affect another job.
                cache.release(connection,false).await;
                run
            }.await;
            let state=match &outcome {Ok(SyncResult{cancelled:true,..})=>"cancelled",Ok(_)=>"succeeded",Err(_)=>"failed"};
            index.set_job_state(&job_id,state,outcome.as_ref().err().map(String::as_str)).await.map_err(|e|e.to_string())?;
            let failed=outcome.is_err(); let was_cancelled=state=="cancelled";
            tokio::spawn(async move {let _=crate::notifications::announce(failed,was_cancelled,"Storage redistribution").await;});
            outcome
        });
        SyncJob {
            cancel,
            events,
            live,
            result,
        }
    }
}
