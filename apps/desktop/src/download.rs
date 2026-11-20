//! The "Download" modal: runs a download with live progress, can be cancelled
//! and ends with an honest result.

use crate::interaction::{ControlFeedback as _, button};

use std::path::PathBuf;
use std::sync::Arc;

use bufonidae_core::get::{Progress as GetProgress, Report};
use gpui::{
    Context, EventEmitter, IntoElement, ParentElement as _, Render, Styled as _, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, StyledExt as _, button::ButtonVariants as _,
};

use crate::backend::TransferEvent;
use crate::backend::{Backend, DownloadJob, DownloadOutcome};
use crate::{format, motion};

const MAX_LISTED: usize = 5;

pub enum DownloadEvent {
    Closed,
}

#[derive(Default)]
struct Counters {
    files_total: usize,
    files_done: usize,
    bytes_total: u64,
    bytes_done: u64,
    current: String,
    cancelling: bool,
    stage: &'static str,
    progress_from: f32,
    progress_to: f32,
    progress_revision: usize,
    progress_started: Option<std::time::Instant>,
    live: Option<bufonidae_rclone::rcd::TransferStats>,
}

enum Phase {
    Running(Counters),
    Finished(Report),
    Cancelled(Counters),
    Failed(String),
}

pub struct DownloadForm {
    out: PathBuf,
    phase: Phase,
    cancel_job: Option<tokio::sync::oneshot::Sender<()>>,
}

impl EventEmitter<DownloadEvent> for DownloadForm {}

impl DownloadForm {
    pub fn new_selection(
        backend: &Arc<Backend>,
        paths: Vec<String>,
        out: PathBuf,
        cx: &mut Context<Self>,
    ) -> Self {
        let DownloadJob {
            mut events,
            cancel,
            result,
        } = backend.download_selection(paths, out.clone());

        cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                this.update(cx, |this, cx| {
                    if let Phase::Running(c) = &mut this.phase {
                        let event = match event {
                            TransferEvent::Stage(label) => {
                                c.stage = label;
                                cx.notify();
                                return;
                            }
                            TransferEvent::Live(sample) => {
                                c.live = sample;
                                if let Some(sample) = sample {
                                    c.progress_from =
                                        c.progress_started.map_or(c.progress_to, |started| {
                                            motion::progress_sample(
                                                c.progress_from,
                                                c.progress_to,
                                                started.elapsed(),
                                            )
                                        });
                                    c.progress_to = c.progress_to.max(
                                        motion::completion_percent(
                                            sample.bytes,
                                            c.bytes_total,
                                            0,
                                            0,
                                        )
                                        .min(99.0),
                                    );
                                    c.progress_revision += 1;
                                    c.progress_started = Some(std::time::Instant::now());
                                }
                                cx.notify();
                                return;
                            }
                            TransferEvent::Progress(event) => event,
                        };
                        c.progress_from = c.progress_started.map_or(c.progress_to, |started| {
                            motion::progress_sample(
                                c.progress_from,
                                c.progress_to,
                                started.elapsed(),
                            )
                        });
                        match event {
                            GetProgress::Planned { files, bytes } => {
                                c.files_total = files;
                                c.bytes_total = bytes;
                            }
                            GetProgress::Downloaded { path, size, .. } => {
                                c.files_done += 1;
                                c.bytes_done += size;
                                c.current = path;
                            }
                            GetProgress::Skipped { path } | GetProgress::Failed { path, .. } => {
                                c.files_done += 1;
                                c.current = path;
                            }
                        }
                        c.progress_to = c.progress_to.max(
                            motion::completion_percent(
                                c.bytes_done,
                                c.bytes_total,
                                c.files_done,
                                c.files_total,
                            )
                            .min(99.0),
                        );
                        c.progress_revision += 1;
                        c.progress_started = Some(std::time::Instant::now());
                    }
                    cx.notify();
                })
                .ok();
            }
            let result = match result.await {
                Ok(result) => result,
                Err(join_error) => Err(join_error.to_string()),
            };
            this.update(cx, |this, cx| {
                let counters =
                    match std::mem::replace(&mut this.phase, Phase::Failed(String::new())) {
                        Phase::Running(c) => c,
                        _ => Counters::default(),
                    };
                this.phase = match result {
                    Ok(DownloadOutcome::Done(report)) => Phase::Finished(report),
                    Ok(DownloadOutcome::Cancelled) => Phase::Cancelled(counters),
                    Err(message) => Phase::Failed(message),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();

        Self {
            out,
            phase: Phase::Running(Counters::default()),
            cancel_job: Some(cancel),
        }
    }

    /// Closes the dialog, except while the download runs.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.phase, Phase::Running(_)) {
            cx.emit(DownloadEvent::Closed);
        }
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        if let Phase::Running(c) = &mut self.phase
            && !c.cancelling
        {
            c.cancelling = true;
            if let Some(tx) = self.cancel_job.take() {
                let _ = tx.send(());
            }
            cx.notify();
        }
    }

    fn line(icon: IconName, text: String, color: gpui::Hsla) -> impl IntoElement {
        div()
            .h_flex()
            .w_full()
            .items_start()
            .gap_2()
            .text_sm()
            .text_color(color)
            .child(Icon::new(icon).size_4().flex_none())
            .child(div().flex_1().min_w(px(0.0)).child(text))
    }
}

impl Render for DownloadForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, danger, success) = (theme.muted_foreground, theme.danger, theme.success);
        let card = || crate::ui::Dialog::new(480.0, window, cx);
        let title = |text: &str| div().text_xl().font_semibold().child(text.to_owned());
        let close = |cx: &mut Context<Self>| {
            div().h_flex().justify_end().child(
                button("close")
                    .primary()
                    .label("Done")
                    .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
            )
        };

        let content = match &self.phase {
            Phase::Running(c) => {
                card()
                    .child(title(if c.cancelling {
                        "Stopping…"
                    } else {
                        "Downloading"
                    }))
                    .child(motion::activity(if c.cancelling { "Cancelling — waiting for the transfer to stop" } else if c.stage.is_empty() { "Preparing transfer" } else { c.stage }, cx))
                    .children((c.files_total > 0).then(|| motion::progress(c.progress_from, c.progress_to, c.progress_revision, cx)))
                    .child(div().text_sm().text_color(muted).child(if c.live.is_some() { "Live transferred bytes include retries. Completion follows file verification." } else { "Waiting for live transfer statistics; completed files are still counted." }))
                    .children(c.live.map(|sample| div().text_sm().text_color(muted).child(format!("{} transferred · {}/s", format::bytes(sample.bytes), format::bytes(sample.speed.max(0.0) as u64)))))
                .child(div().text_color(muted).child(format!(
                        "{} of {} · {} of {}",
                        format::files(c.files_done),
                        format::files(c.files_total),
                        format::bytes(c.bytes_done),
                        format::bytes(c.bytes_total),
                    )))
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .text_color(muted)
                            .child(if c.current.is_empty() { String::new() } else { format!("Last processed: {}", c.current) }),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .text_color(muted)
                            .child(format!("Saving to {}", self.out.display())),
                    )
                    .footer(
 div().h_flex().justify_end().child(
                            button("stop")
                                .label(if c.cancelling { "Stopping…" } else { "Cancel" })
                                .disabled_feedback(c.cancelling)
                                .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                        ),
                    )
                    .into_any_element()
            }

            Phase::Cancelled(c) => card()
                .child(title("Cancelled"))
                .child(Self::line(
                    IconName::Info,
                    format!(
                        "{} of {} were processed before stopping.",
                        c.files_done, c.files_total
                    ),
                    muted,
                ))
                .child(Self::line(
                    IconName::TriangleAlert,
                    format!(
                        "Completed files are kept in {}. The cancelled download was not published at its final name.",
                        self.out.display()
                    ),
                    muted,
                ))
                .footer(close(cx))
                .into_any_element(),

            Phase::Failed(message) => card()
                .child(title("Could not download"))
                .child(Self::line(
                    IconName::TriangleAlert,
                    message.clone(),
                    danger,
                ))
                .footer(close(cx))
                .into_any_element(),

            Phase::Finished(report) => {
                let complete = report.is_complete();
                let n = report.downloaded.len();
                let mut lines: Vec<gpui::AnyElement> = Vec::new();
                let mut push = |icon, text: String, color| {
                    lines.push(Self::line(icon, text, color).into_any_element());
                };
                push(
                    if complete {
                        IconName::CircleCheck
                    } else {
                        IconName::TriangleAlert
                    },
                    format!("{} saved to {}", format::files(n), self.out.display()),
                    if complete { success } else { muted },
                );
                if !report.already_there.is_empty() {
                    push(
                        IconName::Info,
                        format!(
                            "{} already there, left as they were",
                            format::files(report.already_there.len())
                        ),
                        muted,
                    );
                }
                for (label, why) in &report.unreachable {
                    push(
                        IconName::TriangleAlert,
                        format!("Account {label} could not be reached and was not used: {why}"),
                        danger,
                    );
                }
                for (path, reason) in report.failed.iter().take(MAX_LISTED) {
                    push(IconName::CircleX, format!("{path}: {reason}"), danger);
                }
                let more = report.failed.len().saturating_sub(MAX_LISTED);
                if more > 0 {
                    push(IconName::Info, format!("and {more} more problems"), muted);
                }
                card()
                    .child(title(if complete { "Done" } else { "Downloaded with problems" }))
                    .child(div().v_flex().gap_2().children(lines))
                    .footer(close(cx))
                    .into_any_element()
            }
        };
        if matches!(
            self.phase,
            Phase::Finished(_) | Phase::Cancelled(_) | Phase::Failed(_)
        ) {
            motion::fade(content, "download-result", false, 1.0, cx)
        } else {
            content
        }
    }
}
