//! Storage distribution, a read-only preview, then an explicitly approved sync.
use crate::{
    backend::{AccountHealth, Backend, HealthStatus, Snapshot, SyncPreview},
    format,
    interaction::{ControlFeedback as _, button, checkbox},
    motion,
};
use bufonidae_core::redistribute::{Mode, Options, Progress};
use gpui::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window, div, px,
    uniform_list,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, StyledExt as _,
    button::ButtonVariants as _,
    input::{Input, InputEvent, InputState},
    progress::Progress as Bar,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

pub enum DistributionEvent {
    Started,
    Finished,
    OpenCheck,
    Health(Vec<AccountHealth>),
}
impl EventEmitter<DistributionEvent> for DistributionView {}

pub struct DistributionView {
    backend: Arc<Backend>,
    snapshot: Option<Snapshot>,
    health: Vec<AccountHealth>,
    loading: bool,
    planning: bool,
    running: bool,
    mode: Mode,
    inputs: Vec<Entity<InputState>>,
    preview: Option<SyncPreview>,
    approved: bool,
    message: Option<String>,
    error: bool,
    sync_failed: bool,
    cancel: Option<Arc<AtomicBool>>,
    progress: Option<Progress>,
    live: Option<bufonidae_rclone::rcd::TransferStats>,
    started: Option<Instant>,
    detail: Option<usize>,
    progress_from: f32,
    progress_to: f32,
    progress_revision: usize,
    progress_updated: Instant,
}
const FIELDS: [(&str, &str); 8] = [
    ("Download cap (Mbps)", "80"),
    ("Upload cap (Mbps)", "40"),
    ("Setup latency per operation (ms)", "100"),
    ("Quota reset delay (hours)", "0"),
    ("Remote reserve (%)", "5"),
    ("Preview download budget (GiB; 0 = unknown)", "0"),
    ("Mirror copies (2–8)", "2"),
    ("Local staging folder", ""),
];
impl DistributionView {
    pub fn new(backend: Arc<Backend>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let inputs = FIELDS
            .iter()
            .enumerate()
            .map(|(i, (label, value))| {
                let input = cx.new(|cx| {
                    let mut state = InputState::new(window, cx).placeholder(*label);
                    state.set_value(
                        if i == 7 {
                            std::env::temp_dir().to_string_lossy().into_owned()
                        } else {
                            (*value).into()
                        },
                        window,
                        cx,
                    );
                    state
                });
                cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.invalidate();
                        cx.notify();
                    }
                })
                .detach();
                input
            })
            .collect();
        Self {
            backend,
            snapshot: None,
            health: vec![],
            loading: false,
            planning: false,
            running: false,
            mode: Mode::Balance,
            inputs,
            preview: None,
            approved: false,
            message: None,
            error: false,
            sync_failed: false,
            cancel: None,
            progress: None,
            live: None,
            started: None,
            detail: None,
            progress_from: 0.0,
            progress_to: 0.0,
            progress_revision: 0,
            progress_updated: Instant::now(),
        }
    }
    fn busy(&self) -> bool {
        self.loading || self.planning || self.running
    }
    fn invalidate(&mut self) {
        if !self.running {
            self.preview = None;
            self.approved = false;
            self.detail = None;
            self.message = None;
            self.sync_failed = false;
        }
    }
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        self.invalidate();
        self.loading = true;
        self.error = false;
        let data = self.backend.load();
        let health = self.backend.check_health();
        cx.spawn(async move |this, cx| {
            let data = data.await.map_err(|e| e.to_string()).and_then(|r| r);
            let health = health.await.map_err(|e| e.to_string()).and_then(|r| r);
            this.update(cx, |this, cx| {
                this.loading = false;
                match (data, health) {
                    (Ok(data), Ok(health)) => {
                        this.snapshot = Some(data);
                        this.health = health;
                        cx.emit(DistributionEvent::Health(this.health.clone()));
                    }
                    (data, health) => {
                        if let Ok(data) = data.as_ref() {
                            this.snapshot = Some(data.clone());
                        }
                        this.message = Some(
                            data.err()
                                .or_else(|| health.err())
                                .unwrap_or_else(|| "Storage unavailable".into()),
                        );
                        this.error = true;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
    fn number(&self, i: usize, min: f64, max: f64, cx: &Context<Self>) -> Result<f64, String> {
        let value = self.inputs[i]
            .read(cx)
            .value()
            .trim()
            .parse::<f64>()
            .map_err(|_| format!("{}: enter a number using a decimal point.", FIELDS[i].0))?;
        if !value.is_finite() || value < min || value > max {
            return Err(format!(
                "{}: enter a value from {min} to {max}.",
                FIELDS[i].0
            ));
        }
        Ok(value)
    }
    fn options(&self, cx: &Context<Self>) -> Result<(Options, PathBuf), String> {
        let mode = if matches!(self.mode, Mode::Mirror(_)) {
            let n = self.number(6, 2.0, 8.0, cx)?;
            if n.fract() != 0.0 {
                return Err("Mirror copies must be a whole number.".into());
            }
            Mode::Mirror(n as u8)
        } else {
            self.mode.clone()
        };
        let reserve = self.number(4, 0.0, 50.0, cx)?;
        if reserve.fract() != 0.0 {
            return Err("Remote reserve must be a whole percentage.".into());
        }
        let budget = self.number(5, 0.0, 1_000_000_000.0, cx)?;
        let options = Options {
            mode,
            download_rate: (self.number(0, 0.001, 1_000_000.0, cx)? * 125_000.0) as u64,
            upload_rate: (self.number(1, 0.001, 1_000_000.0, cx)? * 125_000.0) as u64,
            latency_ms: self.number(2, 0.0, 60_000.0, cx)? as u32,
            quota_wait_seconds: (self.number(3, 0.0, 8760.0, cx)? * 3600.0) as u64,
            headroom_percent: reserve as u8,
            download_budget: (budget > 0.0).then_some((budget * 1_073_741_824.0) as u64),
        };
        Ok((
            options,
            PathBuf::from(self.inputs[7].read(cx).value().as_str()),
        ))
    }
    fn preview(&mut self, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        self.invalidate();
        let (options, staging) = match self.options(cx) {
            Ok(value) => value,
            Err(error) => {
                self.message = Some(error);
                self.error = true;
                cx.notify();
                return;
            }
        };
        self.planning = true;
        self.error = false;
        let job = self.backend.preview_sync(options, staging);
        cx.spawn(async move|this,cx|{
            let result=job.await.map_err(|e|e.to_string()).and_then(|r|r);
            this.update(cx,|this,cx|{this.planning=false;match result {
                Ok(preview)=>{this.message=Some(if preview.plan.changes.is_empty(){"Already matches the selected distribution. No files need to move.".into()}else{"Preview ready. Review the account projections and file changes, then approve Sync.".into()});this.preview=Some(preview);this.error=false;},
                Err(error)=>{this.message=Some(error);this.error=true;}
            }cx.notify();}).ok();
        }).detach();
        cx.notify();
    }
    fn start(&mut self, cx: &mut Context<Self>) {
        if self.busy() || !self.approved {
            return;
        }
        let Some(preview) = self.preview.clone().filter(|p| !p.plan.changes.is_empty()) else {
            return;
        };
        let total_bytes = preview
            .plan
            .download_bytes
            .saturating_add(preview.plan.upload_bytes);
        let mut job = self.backend.run_sync(preview);
        self.running = true;
        cx.emit(DistributionEvent::Started);
        self.cancel = Some(Arc::clone(&job.cancel));
        self.started = Some(Instant::now());
        self.progress = None;
        self.live = None;
        self.progress_from = 0.0;
        self.progress_to = 0.0;
        self.progress_revision += 1;
        self.progress_updated = Instant::now();
        self.message = None;
        cx.spawn(async move |this, cx| {
            loop {
                gpui::Timer::after(std::time::Duration::from_millis(100)).await;
                let mut progress = None;
                while let Ok(value) = job.events.try_recv() { progress = Some(value); }
                let mut live = None;
                while let Ok(value) = job.live.try_recv() { live = Some(value); }
                if this.update(cx, |this, cx| {
                    let changed = progress.is_some() || live.is_some();
                    if let Some(progress) = progress { this.progress = Some(progress); }
                    if let Some(live) = live { this.live = live; }
                    if changed {
                        let (completed, total) = this.progress.as_ref().map(|p| (p.completed, p.total)).unwrap_or_default();
                        let next = if let Some(stats) = &this.live {
                            motion::completion_percent(stats.bytes, total_bytes, completed, total)
                        } else { motion::completion_percent(0, 0, completed, total) }.min(99.0);
                        if (next - this.progress_to).abs() > f32::EPSILON {
                            this.progress_from = motion::progress_sample(this.progress_from, this.progress_to, this.progress_updated.elapsed());
                            this.progress_to = next;
                            this.progress_updated = Instant::now();
                            this.progress_revision += 1;
                        }
                        cx.notify();
                    }
                }).is_err() { break; }
                if job.result.is_finished() { break; }
            }
            let result = job.result.await.map_err(|e| e.to_string()).and_then(|r| r);
            this.update(cx, |this, cx| {
                this.running = false;
                this.cancel = None;
                this.preview = None;
                this.approved = false;
                let sync_failed = result.is_err();
                this.message = Some(match result {
                    Ok(result) => {
                        this.error = false;
                        format!("{}: {} of {} files synchronized. Completed files remain committed.", if result.cancelled { "Stopped at a safe file boundary" } else { "Sync complete" }, result.completed, result.planned)
                    }
                    Err(error) => {
                        this.error = true;
                        format!("Sync stopped: {error}. Originals are retained unless their verified replacements were already committed. Open Check before creating another preview.")
                    }
                });
                cx.emit(DistributionEvent::Finished);
                let saved_message = this.message.take();
                let saved_error = this.error;
                this.refresh(cx);
                this.message = saved_message;
                this.error = saved_error;
                this.sync_failed = sync_failed;
                cx.notify();
            }).ok();
        }).detach();
        cx.notify();
    }
    fn field(&self, i: usize) -> impl IntoElement {
        div()
            .v_flex()
            .gap_1()
            .w(if i == 7 { px(400.0) } else { px(230.0) })
            .max_w_full()
            .child(div().text_sm().child(FIELDS[i].0))
            .child(Input::new(&self.inputs[i]).disabled(self.busy()))
    }
}
impl Render for DistributionView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let busy = self.busy();
        let mut body=div().v_flex().w_full().min_w(px(0.0)).p_6().gap_4()
            .child(div().h_flex().w_full().gap_3().justify_between()
                .child(div().v_flex().flex_1().min_w(px(0.0)).gap_1().child(div().text_2xl().font_semibold().child("Storage distribution"))
                    .child(div().text_sm().text_color(muted).child("Account quotas, existing replicas and a reviewed plan for redistribution through this PC.")))
                .child(button("refresh-distribution").label("Refresh").disabled_feedback(busy).on_click(cx.listener(|this,_,_,cx|this.refresh(cx)))));
        if self.loading {
            body = body.child(motion::activity("Checking account distribution", cx));
        }
        if let Some(snapshot) = &self.snapshot {
            let logical = snapshot.total_bytes();
            let physical = snapshot.files.iter().fold(0u64, |sum, file| {
                sum.saturating_add(file.size.saturating_mul(file.accounts.len() as u64))
            });
            body = body.child(div().text_sm().child(format!(
                "{} · {} logical · {} indexed replica bytes. Remote usage can include other files.",
                format::files(snapshot.files.len()),
                format::bytes(logical),
                format::bytes(physical)
            )));
            if snapshot.accounts.is_empty() {
                body =
                    body.child("Connect accounts in Accounts to see storage and configure Sync.");
            }
            for account in &snapshot.accounts {
                let health = self.health.iter().find(|h| h.id == account.id);
                let quota = self
                    .preview
                    .as_ref()
                    .and_then(|p| p.plan.quotas.iter().find(|q| q.id == account.id))
                    .map(|q| (q.total, q.used))
                    .or_else(|| {
                        health.and_then(|h| match h.status {
                            HealthStatus::Ok {
                                total: Some(total),
                                used,
                                free,
                            } if total > 0 => used
                                .or_else(|| free.map(|f| total.saturating_sub(f)))
                                .map(|used| (total, used)),
                            _ => None,
                        })
                    });
                let (files, managed) = snapshot.usage(&account.id);
                let mut row = div()
                    .v_flex()
                    .w_full()
                    .min_w(px(0.0))
                    .gap_2()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().font_medium().child(account.label.clone()));
                if let Some((total, used)) = quota {
                    row = row
                        .child(div().text_sm().child(format!(
                            "Last check · {} / {} used · {} available",
                            format::bytes(used),
                            format::bytes(total),
                            format::bytes(total.saturating_sub(used))
                        )))
                        .child(
                            Bar::new().w_full().value(
                                (used as f64 / total as f64 * 100.0).clamp(0.0, 100.0) as f32,
                            ),
                        );
                } else {
                    row = row.child(
                        div()
                            .h_flex()
                            .items_start()
                            .gap_2()
                            .text_sm()
                            .text_color(cx.theme().danger)
                            .child(Icon::new(IconName::TriangleAlert).size_4().flex_none())
                            .child(div().flex_1().min_w(px(0.0)).child(
                                match health.map(|h| &h.status) {
                                    Some(HealthStatus::Unreachable(why)) => {
                                        format!("Unavailable: {why}")
                                    }
                                    _ => "Quota not reported. Sync requires known quotas.".into(),
                                },
                            )),
                    );
                }
                if let Some(projected) = self
                    .preview
                    .as_ref()
                    .and_then(|p| p.plan.projected.iter().find(|q| q.id == account.id))
                {
                    row = row
                        .child(div().text_sm().child(format!(
                            "After Sync · {} used · {} available",
                            format::bytes(projected.used),
                            format::bytes(projected.total.saturating_sub(projected.used))
                        )))
                        .child(
                            Bar::new().w_full().value(
                                (projected.used as f64 / projected.total as f64 * 100.0)
                                    .clamp(0.0, 100.0) as f32,
                            ),
                        );
                }
                if let Some(preview) = &self.preview {
                    let down = preview.plan.changes.iter().fold(0u64, |sum, c| {
                        let reads = usize::from(c.sources.first() == Some(&account.id))
                            + usize::from(c.add.contains(&account.id))
                            + usize::from(c.remove.contains(&account.id));
                        sum.saturating_add(c.size.saturating_mul(reads as u64))
                    });
                    let up = preview
                        .plan
                        .changes
                        .iter()
                        .filter(|c| c.add.contains(&account.id))
                        .fold(0u64, |sum, c| sum.saturating_add(c.size));
                    row = row.child(div().text_sm().child(format!(
                        "Planned traffic · {} download · {} upload (before retries/fallbacks)",
                        format::bytes(down),
                        format::bytes(up)
                    )));
                }
                body = body.child(row.child(div().text_sm().text_color(muted).child(format!(
                    "{} indexed files · {} indexed on this account",
                    files,
                    format::bytes(managed)
                ))));
            }
        }
        body = body.child(div().text_lg().font_semibold().child("Sync strategy"));
        let mut modes = div().h_flex().flex_wrap().gap_2();
        for (id, label, mode) in [
            ("mode-balance", "Balance utilization", Mode::Balance),
            ("mode-repair", "Repair missing replicas", Mode::Repair),
            ("mode-mirror", "Mirror · RAID 1 style", Mode::Mirror(2)),
            (
                "mode-evacuate",
                "Evacuate an account",
                Mode::Evacuate(String::new()),
            ),
        ] {
            let selected = std::mem::discriminant(&self.mode) == std::mem::discriminant(&mode);
            let mode_button = button(id)
                .label(label)
                .selected(selected)
                .disabled_feedback(busy)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.mode = mode.clone();
                    if matches!(this.mode, Mode::Evacuate(_)) {
                        this.mode = Mode::Evacuate(
                            this.snapshot
                                .as_ref()
                                .and_then(|s| s.accounts.first())
                                .map(|a| a.id.clone())
                                .unwrap_or_default(),
                        );
                    }
                    this.invalidate();
                    cx.notify();
                }));
            modes = modes.child(if selected {
                mode_button.primary()
            } else {
                mode_button
            });
        }
        let strategy_hint = match self.mode {
            Mode::Balance => "Even out account usage while keeping existing copies.",
            Mode::Repair => "Add missing copies to match your policy.",
            Mode::Mirror(_) => "Keep 2–8 copies on separate accounts.",
            Mode::Evacuate(_) => "Move indexed copies off the selected account.",
        };
        body = body
            .child(modes)
            .child(div().text_sm().text_color(muted).child(strategy_hint));
        if matches!(self.mode, Mode::Evacuate(_)) {
            let mut targets = div().h_flex().flex_wrap().gap_2();
            if let Some(snapshot) = &self.snapshot {
                for (i, account) in snapshot.accounts.iter().enumerate() {
                    let id = account.id.clone();
                    let chosen = matches!(&self.mode, Mode::Evacuate(selected) if selected == &id);
                    let target_button = button(("evacuate-account", i))
                        .label(format!("Evacuate {}", account.label))
                        .selected(chosen)
                        .disabled_feedback(busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.mode = Mode::Evacuate(id.clone());
                            this.invalidate();
                            cx.notify();
                        }));
                    targets = targets.child(if chosen {
                        target_button.primary()
                    } else {
                        target_button
                    });
                }
            }
            body=body.child(targets).child(div().text_sm().child("Only indexed files move. Other remote data stays on the evacuated account; it remains connected."));
        }
        let mut fields = div().h_flex().flex_wrap().items_end().gap_4();
        for i in 0..8 {
            if i != 6 || matches!(self.mode, Mode::Mirror(_)) {
                fields = fields.child(self.field(i));
            }
        }
        body = body.child(fields).child(
            div()
                .v_flex()
                .gap_1()
                .text_sm()
                .text_color(muted)
                .child("Caps limit rclone speeds. The preview uses your quota delay and download budget.")
                .child("Sync verifies replacement copies before removing originals.")
                .child("Avoid changing remote files while Sync runs."),
        );
        if let Some(message) = &self.message {
            body = body.child(
                div()
                    .text_color(if self.error {
                        cx.theme().danger
                    } else {
                        cx.theme().foreground
                    })
                    .child(bufonidae_rclone::escape_control(message)),
            );
        }
        if self.sync_failed {
            body = body.child(
                button("check-after-sync")
                    .primary()
                    .label("Open Check")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(DistributionEvent::OpenCheck))),
            );
        } else {
            body = body.child(
                button("preview-sync")
                    .primary()
                    .label(if self.planning {
                        "Planning…"
                    } else {
                        "Preview Sync"
                    })
                    .loading(self.planning && motion::enabled(cx))
                    .disabled_feedback(busy)
                    .on_click(cx.listener(|this, _, _, cx| this.preview(cx))),
            );
        }
        if self.planning {
            body = body.child(motion::activity("Preparing distribution preview", cx));
        }
        if let Some(preview) = &self.preview {
            let p = &preview.plan;
            let source_download = p.changes.iter().map(|c| c.size).sum::<u64>();
            let readback = p
                .changes
                .iter()
                .map(|c| c.size.saturating_mul(c.add.len() as u64))
                .sum::<u64>();
            let recheck = p
                .download_bytes
                .saturating_sub(source_download)
                .saturating_sub(readback);
            body=body.child(div().text_lg().font_semibold().child(if self.running {"Reviewed plan · Sync running"}else{"Preview · no remote changes yet"}))
                .child(format!("{} file changes · {:.2} hours at configured caps, latency and quota delay",p.changes.len(),p.seconds()/3600.0))
                .child(format!("Download {} = {} source + {} destination readback + {} original rechecks. Upload {}.",format::bytes(p.download_bytes),format::bytes(source_download),format::bytes(readback),format::bytes(recheck),format::bytes(p.upload_bytes)))
                .child(format!("Peak local staging: {} + 64 MiB reserve · {} available in {}",format::bytes(p.temporary_bytes),format::bytes(preview.local_free),preview.staging.display()))
                .child(div().text_sm().text_color(muted).child("Estimate uses transfer caps, setup time and quota delay. Retries and throttling can extend it."));
            let transfer_seconds = p.download_bytes as f64 / p.options.download_rate as f64
                + p.upload_bytes as f64 / p.options.upload_rate as f64;
            body = body.child(div().text_sm().child(format!("50%-of-cap scenario: {:.2} hours. This is a speed scenario, not a measured confidence interval.",(p.seconds()+transfer_seconds)/3600.0)));
            if p.warnings.len() <= 2 {
                for warning in &p.warnings {
                    body = body.child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(bufonidae_rclone::escape_control(warning)),
                    );
                }
            } else {
                let first = bufonidae_rclone::escape_control(&p.warnings[0]);
                let summary = if p.warnings.iter().all(|w| w.contains("hash")) {
                    format!(
                        "{} files have no trusted hash baseline (e.g. {first}).",
                        p.warnings.len()
                    )
                } else {
                    format!(
                        "{} warnings before sync (first: {first}).",
                        p.warnings.len()
                    )
                };
                body = body.child(div().text_sm().text_color(muted).child(summary));
            }
            let changes = p.changes.clone();
            let names = p
                .quotas
                .iter()
                .map(|q| (q.id.clone(), q.label.clone()))
                .collect::<std::collections::BTreeMap<_, _>>();
            if !changes.is_empty() {
                let total = changes.len();
                let visible = total.min(3);
                let scope = if total > visible {
                    format!("Showing 1–{visible} of {total} · Scroll list to review")
                } else {
                    format!("Showing {total} of {total}")
                };
                body = body
                    .child(
                        div()
                            .h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(div().text_sm().font_semibold().child("Planned changes"))
                            .child(div().text_sm().text_color(muted).child(scope)),
                    )
                    .child(
                        div()
                            .w_full()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .px_3()
                            .child(
                                uniform_list(
                                    "sync-preview-files",
                                    changes.len(),
                                    cx.processor(
                                        move |_this, range: std::ops::Range<usize>, _window, cx| {
                                            range
                                                .filter_map(|i| changes.get(i).map(|c| (i, c)))
                                                .map(|(i, c)| {
                                                    let labels = |ids: &[String]| {
                                                        ids.iter()
                                                            .map(|id| {
                                                                names
                                                                    .get(id)
                                                                    .cloned()
                                                                    .unwrap_or_else(|| id.clone())
                                                            })
                                                            .collect::<Vec<_>>()
                                                            .join(", ")
                                                    };
                                                    div()
                                                        .h_flex()
                                                        .w_full()
                                                        .min_w(px(0.0))
                                                        .h(px(88.0))
                                                        .gap_3()
                                                        .items_center()
                                                        .border_b_1()
                                                        .border_color(cx.theme().border)
                                                        .child(
                                                            div()
                                                                .v_flex()
                                                                .flex_1()
                                                                .min_w(px(0.0))
                                                                .gap_1()
                                                                .child(div().truncate().child(
                                                                    bufonidae_rclone::escape_control(
                                                                        &c.path,
                                                                    ),
                                                                ))
                                                                .child(
                                                                    div().text_sm().truncate().child(
                                                                        format!(
                                                                            "{} · add [{}] · retire [{}]",
                                                                            format::bytes(c.size),
                                                                            labels(&c.add),
                                                                            labels(&c.remove)
                                                                        ),
                                                                    ),
                                                                ),
                                                        )
                                                        .child(
                                                            button(("sync-file-detail", i))
                                                                .label("Details")
                                                                .on_click(cx.listener(
                                                                    move |this, _, _, cx| {
                                                                        this.detail = Some(i);
                                                                        cx.notify();
                                                                    },
                                                                )),
                                                        )
                                                })
                                                .collect::<Vec<_>>()
                                        },
                                    ),
                                )
                                .w_full()
                                .h(px(264.0)),
                            ),
                    );
            }
            if let Some(c) = self.detail.and_then(|i| p.changes.get(i)) {
                let account_labels = |ids: &[String]| {
                    if ids.is_empty() {
                        return "—".to_owned();
                    }
                    ids.iter()
                        .map(|id| {
                            p.quotas
                                .iter()
                                .find(|quota| quota.id == *id)
                                .map_or(id.as_str(), |quota| quota.label.as_str())
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let detail_row = |label: &'static str, value: String| {
                    div()
                        .h_flex()
                        .w_full()
                        .gap_3()
                        .text_sm()
                        .child(
                            div()
                                .w(px(120.0))
                                .flex_none()
                                .text_color(muted)
                                .child(label),
                        )
                        .child(div().flex_1().min_w(px(0.0)).child(value))
                };
                body = body.child(
                    div()
                        .v_flex()
                        .w_full()
                        .gap_2()
                        .p_3()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().secondary.opacity(0.35))
                        .child(
                            div()
                                .h_flex()
                                .w_full()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .font_semibold()
                                        .truncate()
                                        .child(bufonidae_rclone::escape_control(&c.path)),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .text_sm()
                                        .text_color(muted)
                                        .child(format::bytes(c.size)),
                                )
                                .child(
                                    button("close-sync-detail")
                                        .ghost()
                                        .compact()
                                        .label("Close")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.detail = None;
                                            cx.notify();
                                        })),
                                ),
                        )
                        .child(detail_row("Sources", account_labels(&c.sources)))
                        .child(detail_row("New accounts", account_labels(&c.add)))
                        .child(detail_row("Retired accounts", account_labels(&c.remove))),
                );
            }
            body=body.child(checkbox("approve-sync").label("I reviewed the plan and paused external edits. Sync may retire old copies only after their replacements are read back and verified.").checked(self.approved).disabled_feedback(busy||p.changes.is_empty()).on_click(cx.listener(|this,value:&bool,_,cx|{this.approved = *value;cx.notify();})))
                .child(button("execute-sync").primary().label("Sync reviewed plan").disabled_feedback(busy||!self.approved||p.changes.is_empty()).on_click(cx.listener(|this,_,_,cx|this.start(cx))));
        }
        if self.running {
            let stopping = self
                .cancel
                .as_ref()
                .is_some_and(|cancel| cancel.load(Ordering::Relaxed));
            body = body.child(motion::activity(
                if stopping {
                    "Stopping after the current file"
                } else {
                    "Synchronizing storage"
                },
                cx,
            ));
            if let Some(progress) = &self.progress {
                body = body
                    .child(format!(
                        "{} / {} files · {} · {}",
                        progress.completed,
                        progress.total,
                        progress.stage,
                        bufonidae_rclone::escape_control(&progress.path)
                    ))
                    .child(div().w_full().child(motion::progress(
                        self.progress_from,
                        self.progress_to,
                        self.progress_revision,
                        cx,
                    )));
            }
            if let Some(live) = &self.live {
                body = body.child(format!(
                    "{} transferred · {}/s · {:.1} min elapsed",
                    format::bytes(live.bytes),
                    format::bytes(live.speed.max(0.0) as u64),
                    self.started
                        .map(|t| t.elapsed().as_secs_f64() / 60.0)
                        .unwrap_or_default()
                ));
            }
            body=body.child(button("stop-sync").label("Stop after current file").disabled_feedback(stopping).on_click(cx.listener(|this,_,_,cx|{if let Some(cancel)=&this.cancel{cancel.store(true,Ordering::Relaxed);}cx.notify();})))
                .child(div().text_sm().child("Stopping waits for the current file's copy/verify/commit sequence; a large file can take hours. Completed files remain saved. Keep this app open. Failures and interrupted runs are recorded in Transfers; use Check before a new preview."));
        }
        div()
            .id("storage-distribution")
            .size_full()
            .min_w(px(0.0))
            .overflow_y_scroll()
            .child(body)
    }
}
