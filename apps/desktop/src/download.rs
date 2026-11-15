//! The "Download" modal: runs a download with live progress, can be cancelled
//! and ends with an honest result.

use std::path::PathBuf;
use std::sync::Arc;

use bufonidae_core::get::{Progress as GetProgress, Report};
use gpui::{
    Context, EventEmitter, IntoElement, ParentElement as _, Render, Styled as _, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, StyledExt as _,
    button::{Button, ButtonVariants as _},
    progress::Progress,
};

use crate::backend::{Backend, DownloadJob, DownloadOutcome};
use crate::format;

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
    pub fn new(backend: &Arc<Backend>, path: String, out: PathBuf, cx: &mut Context<Self>) -> Self {
        let DownloadJob {
            mut events,
            cancel,
            result,
        } = backend.download(path, out.clone());

        cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                this.update(cx, |this, cx| {
                    if let Phase::Running(c) = &mut this.phase {
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
            .items_start()
            .gap_2()
            .text_sm()
            .text_color(color)
            .child(Icon::new(icon).size_4().flex_none())
            .child(div().flex_1().child(text))
    }
}

impl Render for DownloadForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, danger, success, border, surface) = (
            theme.muted_foreground,
            theme.danger,
            theme.success,
            theme.border,
            theme.popover,
        );
        let card = || {
            div()
                .v_flex()
                .w(px(480.0))
                .gap_4()
                .p_6()
                .rounded(px(14.0))
                .bg(surface)
                .border_1()
                .border_color(border)
                .shadow_lg()
        };
        let title = |text: &str| div().text_xl().font_semibold().child(text.to_owned());
        let close = |cx: &mut Context<Self>| {
            div().h_flex().justify_end().child(
                Button::new("close")
                    .primary()
                    .label("Done")
                    .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
            )
        };

        match &self.phase {
            Phase::Running(c) => {
                #[allow(clippy::cast_precision_loss)]
                let percent = if c.bytes_total > 0 {
                    (c.bytes_done as f32 / c.bytes_total as f32 * 100.0).min(100.0)
                } else if c.files_total > 0 {
                    c.files_done as f32 / c.files_total as f32 * 100.0
                } else {
                    0.0
                };
                card()
                    .child(title(if c.cancelling {
                        "Stopping…"
                    } else {
                        "Downloading"
                    }))
                    .child(Progress::new().value(percent))
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
                            .child(c.current.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .text_color(muted)
                            .child(format!("Saving to {}", self.out.display())),
                    )
                    .child(
                        div().h_flex().justify_end().child(
                            Button::new("stop")
                                .label(if c.cancelling { "Stopping…" } else { "Cancel" })
                                .disabled(c.cancelling)
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
                        "The file being downloaded may have left a temporary `.partial` file in {}.",
                        self.out.display()
                    ),
                    muted,
                ))
                .child(close(cx))
                .into_any_element(),

            Phase::Failed(message) => card()
                .child(title("Could not download"))
                .child(Self::line(
                    IconName::TriangleAlert,
                    message.clone(),
                    danger,
                ))
                .child(close(cx))
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
                    .child(close(cx))
                    .into_any_element()
            }
        }
    }
}
