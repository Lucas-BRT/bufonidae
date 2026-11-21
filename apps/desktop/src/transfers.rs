//! Persisted transfer history and explicit recovery actions.

use crate::interaction::{ControlFeedback as _, button};

use std::path::PathBuf;

use bufonidae_core::index::Job;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use gpui_component::button::ButtonVariants as _;
use gpui_component::{ActiveTheme as _, Icon, IconName, StyledExt as _};

use crate::app::{BufonidaeApp, centered};
use crate::backend::HealthStatus;
use crate::commands::Command;
use crate::format;
use crate::ui::{self, Tone};

/// Only the latest jobs are drawn; older ones stay in the index.
const SHOWN: usize = 200;

enum Item {
    Group(&'static str),
    Job(Job),
}

/// Icon, tone and short label for each job state; the label is also the
/// accessible name, so state never depends on color.
fn state_chip(state: &str) -> (Tone, IconName, &'static str) {
    match state {
        "queued" => (Tone::Neutral, IconName::Loader, "Waiting"),
        "running" => (Tone::Neutral, IconName::Loader, "Running"),
        "interrupted" => (Tone::Warning, IconName::TriangleAlert, "Interrupted"),
        "done" => (Tone::Success, IconName::CircleCheck, "Completed"),
        "failed" => (Tone::Danger, IconName::CircleX, "Failed"),
        "cancelled" => (Tone::Neutral, IconName::Minus, "Cancelled"),
        _ => (Tone::Neutral, IconName::Info, "Unknown"),
    }
}

fn operation_icon(operation: &str) -> IconName {
    match operation {
        "download" => IconName::ArrowDown,
        "upload" => IconName::ArrowUp,
        _ => IconName::Redo,
    }
}

fn group_of(age_secs: i64) -> &'static str {
    match age_secs {
        i64::MIN..=86_399 => "Last 24 hours",
        86_400..=604_799 => "Previous 7 days",
        _ => "Older",
    }
}

/// Newest first, with a heading wherever the age bucket changes.
fn grouped(jobs: &[Job], now: i64) -> Vec<Item> {
    let mut items = Vec::new();
    let mut current = "";
    for job in jobs.iter().rev().take(SHOWN) {
        let group = group_of(now - job.created_at);
        if group != current {
            current = group;
            items.push(Item::Group(group));
        }
        items.push(Item::Job(job.clone()));
    }
    items
}

fn state_label(state: &str) -> &'static str {
    match state {
        "queued" => "Waiting",
        "running" => "Running",
        "interrupted" => "Interrupted — review before retrying",
        "done" => "Completed",
        "failed" => "Failed — some files may have completed",
        "cancelled" => "Cancelled — completed files are kept",
        _ => "Unknown state",
    }
}

fn upload_request(job: &Job) -> Result<(Vec<PathBuf>, String), String> {
    let payload: serde_json::Value =
        serde_json::from_str(&job.payload).map_err(|e| e.to_string())?;
    let paths: Vec<PathBuf> =
        serde_json::from_value(payload["paths"].clone()).map_err(|e| e.to_string())?;
    let destination: String =
        serde_json::from_value(payload["destination"].clone()).map_err(|e| e.to_string())?;
    if paths.is_empty() || paths.iter().any(|p| !p.is_absolute()) {
        return Err(
            "The saved upload has no valid absolute source paths. Add the files again.".into(),
        );
    }
    Ok((paths, destination))
}

fn download_request(job: &Job) -> Result<(Vec<String>, PathBuf), String> {
    serde_json::from_str::<(Vec<String>, PathBuf)>(&job.payload)
        .or_else(|_| {
            serde_json::from_str::<(String, PathBuf)>(&job.payload)
                .map(|(path, out)| (vec![path], out))
        })
        .map_err(|error| error.to_string())
}

fn summary(job: &Job) -> String {
    if job.operation == "sync" {
        "Storage Sync · review distribution and run Check before retrying".into()
    } else if job.operation == "download" {
        download_request(job)
            .map(|(paths, out)| format!("Download {} → {}", paths.join(", "), out.display()))
            .unwrap_or_else(|_| "Download — saved request unavailable".into())
    } else {
        upload_request(job)
            .map(|(paths, destination)| {
                format!("Upload {} source(s) → /{destination}", paths.len())
            })
            .unwrap_or_else(|_| "Upload — saved request unavailable".into())
    }
}

impl BufonidaeApp {
    pub(crate) fn toggle_transfer_details(
        &mut self,
        id: &str,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        if self.transfer_details.as_deref() == Some(id) {
            self.close_transfer_details(window, cx);
        } else {
            self.transfer_details = Some(id.to_owned());
            window.focus(&self.transfer_panel_focus);
            cx.notify();
        }
    }

    fn close_transfer_details(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        self.transfer_details = None;
        window.focus(&self.root_focus);
        cx.notify();
    }

    pub(crate) fn transfers_screen(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let jobs = self.snapshot().map(|s| s.jobs.clone()).unwrap_or_default();
        let total = jobs.len();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
        let items = grouped(&jobs, now);
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let header = div()
            .h_flex()
            .w_full()
            .min_w(px(0.0))
            .justify_between()
            .items_start()
            .gap_4()
            .px_6()
            .pt_6()
            .pb_4()
            .child(
                div()
                    .v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap_1()
                    .child(div().text_2xl().font_semibold().child("Transfers"))
                    .child(
                        div()
                            .text_color(muted)
                            .truncate()
                            .child("Updates every 2 seconds while open."),
                    ),
            )
            .child(
                button("refresh-transfers")
                    .w(px(112.0))
                    .flex_none()
                    .label(if self.transfers_refreshing {
                        "Refreshing…"
                    } else {
                        "Refresh"
                    })
                    .disabled_feedback(self.transfers_refreshing)
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_transfers(cx))),
            );
        let notices = [
            self.transfer_error.clone(),
            self.transfer_refresh_error.clone(),
        ]
        .into_iter()
        .flatten()
        .map(|message| {
            ui::notice(Tone::Danger, message, None, cx)
                .mx_6()
                .mb_3()
                .into_any_element()
        })
        .collect::<Vec<_>>();

        if items.is_empty() {
            return div()
                .v_flex()
                .size_full()
                .min_w(px(0.0))
                .child(header)
                .children(notices)
                .child(centered(
                    cx,
                    Some(IconName::Inbox),
                    "No transfers yet",
                    Some("Add files from Files, or download a stored file.".to_owned()),
                ));
        }

        let open = self.transfer_details.clone();
        let rows = items
            .into_iter()
            .enumerate()
            .map(|(i, item)| match item {
                Item::Group(label) => div()
                    .px_6()
                    .pt_4()
                    .pb_1()
                    .text_sm()
                    .font_semibold()
                    .text_color(muted)
                    .child(label)
                    .into_any_element(),
                Item::Job(job) => self.transfer_row(i, &job, now, open.as_deref(), cx),
            })
            .collect::<Vec<_>>();
        let more = (total > SHOWN).then(|| {
            div()
                .px_6()
                .py_3()
                .text_sm()
                .text_color(muted)
                .child(format!("Showing the latest {SHOWN} of {total} transfers."))
        });
        div()
            .v_flex()
            .size_full()
            .min_w(px(0.0))
            .child(header)
            .children(notices)
            .child(div().h_px().bg(border))
            .child(
                div()
                    .id("transfer-history")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .child(div().v_flex().w_full().children(rows).children(more)),
            )
    }

    fn transfer_row(
        &self,
        i: usize,
        job: &Job,
        now: i64,
        open: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let (muted, border) = (theme.muted_foreground, theme.border);
        let (tone, icon, label) = state_chip(&job.state);
        let status = if job.state == "running" {
            crate::motion::activity("Running", cx)
        } else {
            ui::chip(tone, Some(icon), label, cx)
        };
        let recoverable = matches!(job.state.as_str(), "interrupted" | "failed" | "cancelled")
            && matches!(job.operation.as_str(), "upload" | "download");
        let action = recoverable.then(|| {
            let job = job.clone();
            let button = button(("review-transfer", i)).flex_none();
            let needs_account = job.operation == "download"
                && self.snapshot().is_some_and(|snapshot| {
                    snapshot.accounts.iter().all(|account| {
                        matches!(
                            self.health.of(&account.id),
                            Some(HealthStatus::Unreachable(_))
                        )
                    })
                });
            let button = if needs_account {
                return button.label("Open Accounts").on_click(cx.listener(
                    |this, _, window, cx| {
                        this.execute_command(Command::Accounts, window, cx);
                    },
                ));
            } else if job.operation == "download" {
                button
                    .ghost()
                    .icon(Icon::empty().path("icons/rotate-cw.svg"))
                    .tooltip("Retry")
            } else {
                button.label("Review upload")
            };
            button.on_click(cx.listener(move |this, _, window, cx| {
                this.transfer_error = None;
                if job.operation == "download" {
                    match download_request(&job) {
                        Ok((path, out)) if out.is_absolute() => {
                            this.open_download_selection(path, out, window, cx);
                        }
                        _ => {
                            this.transfer_error = Some(
                                "The saved destination is invalid. Download the file again \
                                     from Files."
                                    .into(),
                            );
                        }
                    }
                } else {
                    match upload_request(&job) {
                        Ok((paths, destination)) => {
                            this.review_upload(paths, destination, window, cx);
                        }
                        Err(message) => this.transfer_error = Some(message),
                    }
                }
                cx.notify();
            }))
        });
        let is_open = open == Some(job.id.as_str());
        let detail_id = job.id.clone();
        let error = job
            .error
            .as_deref()
            .is_some_and(|error| !error.is_empty())
            .then(|| {
                div()
                    .text_sm()
                    .truncate()
                    .text_color(muted)
                    .child("Error in Details")
            });
        let label = if job.operation == "download" {
            download_request(job).ok().and_then(|(paths, _)| {
                let path = paths.first()?;
                let name = path.rsplit('/').next().unwrap_or(path);
                Some(if paths.len() == 1 {
                    name.to_owned()
                } else {
                    format!("{name} and {} more", paths.len() - 1)
                })
            })
        } else {
            None
        }
        .unwrap_or_else(|| summary(job));
        let label = bufonidae_rclone::escape_control(&label);
        let absolute = format::utc(job.created_at);
        let detail_key_id = detail_id.clone();
        let hover = theme.muted;
        let focus = theme.ring;
        let item = div()
            .id(gpui::SharedString::from(format!("transfer-{}", job.id)))
            .h_flex()
            .flex_1()
            .min_w(px(0.0))
            .items_center()
            .gap_4()
            .rounded_md()
            .px_3()
            .py_2()
            .tab_index(0)
            .cursor_pointer()
            .hover(|style| style.bg(hover))
            .border_1()
            .border_color(gpui::transparent_black())
            .focus(|style| style.border_color(focus))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.toggle_transfer_details(&detail_id, window, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.toggle_transfer_details(&detail_key_id, window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                Icon::new(operation_icon(&job.operation))
                    .flex_none()
                    .text_color(muted),
            )
            .child(
                div()
                    .v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap_1()
                    .child(
                        div()
                            .id(("transfer-name", i))
                            .w(px(340.0))
                            .max_w_full()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .tooltip({
                                let label = label.clone();
                                move |window, cx| {
                                    let label = label.clone();
                                    gpui_component::tooltip::Tooltip::element(move |_, _| {
                                        div().w(px(300.0)).min_w(px(0.0)).child(label.clone())
                                    })
                                    .build(window, cx)
                                }
                            })
                            .child(label),
                    )
                    .child(
                        div().h_flex().gap_2().items_center().child(status).child(
                            div()
                                .id(gpui::SharedString::from(format!("when-{}", job.id)))
                                .text_sm()
                                .text_color(muted)
                                .tooltip(move |window, cx| {
                                    gpui_component::tooltip::Tooltip::new(absolute.clone())
                                        .build(window, cx)
                                })
                                .child(format::ago(now, job.created_at)),
                        ),
                    )
                    .children(error),
            )
            .child(
                Icon::new(if is_open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .flex_none()
                .text_color(muted),
            );
        let line = div()
            .h_flex()
            .w_full()
            .min_w(px(0.0))
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .child(item)
            .children(action);
        let mut row = div()
            .v_flex()
            .w_full()
            .min_w(px(0.0))
            .border_b_1()
            .border_color(border)
            .child(line);
        if is_open {
            row = row.child(crate::motion::reveal(
                self.transfer_details_panel(job, cx),
                ("transfer-reveal", i),
                320.0,
                cx,
            ));
        }
        row.into_any_element()
    }

    fn transfer_details_panel(&self, job: &Job, cx: &mut Context<Self>) -> AnyElement {
        let sources = if job.operation == "upload" {
            upload_request(job)
                .map(|(paths, _)| {
                    paths
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        div()
            .id("transfer-details")
            .track_focus(&self.transfer_panel_focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.close_transfer_details(window, cx);
                    cx.stop_propagation();
                }
            }))
            .v_flex()
            .gap_1()
            .w_full()
            .min_w(px(0.0))
            .max_h(px(300.0))
            .overflow_y_scroll()
            .px_6()
            .pb_4()
            .child(div().font_semibold().child("Transfer details"))
            .child(div().child(bufonidae_rclone::escape_control(&summary(job))))
            .child(div().child(state_label(&job.state)))
            .children(sources.into_iter().map(|path| {
                div()
                    .text_sm()
                    .child(bufonidae_rclone::escape_control(&path))
            }))
            .child(div().text_sm().child(bufonidae_rclone::escape_control(
                job.error.as_deref().unwrap_or("No error was recorded."),
            )))
            .into_any_element()
    }
}
