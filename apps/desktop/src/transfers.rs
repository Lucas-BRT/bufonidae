//! Persisted transfer history and explicit recovery actions.

use crate::interaction::{ControlFeedback as _, button};

use std::path::PathBuf;

use bufonidae_core::index::Job;
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px, uniform_list,
};
use gpui_component::{ActiveTheme as _, StyledExt as _};

use crate::app::BufonidaeApp;

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
    pub(crate) fn transfers_screen(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut jobs = self.snapshot().map(|s| s.jobs.clone()).unwrap_or_default();
        jobs.reverse();
        let muted = cx.theme().muted_foreground;
        let header = div().h_flex().w_full().min_w(px(0.0)).justify_between().gap_4().p_6()
            .child(div().v_flex().flex_1().min_w(px(0.0)).gap_2()
                .child(div().text_2xl().font_semibold().child("Transfers"))
                .child(div().text_color(muted).child("History updates every 2 seconds while this screen is open. Review interrupted uploads before retrying.")))
            .child(button("refresh-transfers").w(px(112.0)).flex_none().label(if self.transfers_refreshing { "Refreshing…" } else { "Refresh" }).disabled_feedback(self.transfers_refreshing)
                .on_click(cx.listener(|this, _, _, cx| this.refresh_transfers(cx))));
        let error = self.transfer_error.clone().map(|message| {
            div()
                .px_6()
                .pb_4()
                .text_color(cx.theme().danger)
                .child(message)
        });
        let refresh_error = self.transfer_refresh_error.clone().map(|message| {
            div()
                .px_6()
                .pb_4()
                .text_color(cx.theme().danger)
                .child(message)
        });
        let details = self
            .transfer_details
            .as_ref()
            .and_then(|id| jobs.iter().find(|job| &job.id == id))
            .map(|job| {
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
                    .w_full()
                    .min_w(px(0.0))
                    .max_h(px(220.0))
                    .overflow_y_scroll()
                    .px_6()
                    .py_4()
                    .border_b_1()
                    .border_color(cx.theme().border)
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
                    .child(
                        button("close-transfer-details")
                            .label("Close details")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.transfer_details = None;
                                cx.notify();
                            })),
                    )
            });
        let empty = jobs.is_empty().then(|| {
            div()
                .p_6()
                .text_color(muted)
                .child("No transfers yet. Add files from Files, or download a stored file.")
        });
        let list = uniform_list("transfer-history", jobs.len(), cx.processor(move |_this, range: std::ops::Range<usize>, _window, cx| {
            range.filter_map(|i| jobs.get(i).cloned().map(|job| (i, job))).map(|(i, job)| {
                let recoverable = matches!(job.state.as_str(), "interrupted" | "failed" | "cancelled") && matches!(job.operation.as_str(), "upload" | "download");
                let action = recoverable.then(|| {
                    let job = job.clone();
                    let label = if job.operation == "download" { "Retry download" } else { "Review upload" };
                    button(("review-transfer", i)).label(label)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.transfer_error = None;
                            if job.operation == "download" {
                                match download_request(&job) {
                                    Ok((path, out)) if out.is_absolute() => this.open_download_selection(path, out, window, cx),
                                    _ => this.transfer_error = Some("The saved destination is invalid. Download the file again from Files.".into()),
                                }
                            } else {
                                match upload_request(&job) {
                                    Ok((paths, destination)) => this.review_upload(paths, destination, window, cx),
                                    Err(message) => this.transfer_error = Some(message),
                                }
                            }
                            cx.notify();
                        }))
                });
                let detail_id = job.id.clone();
                div().h_flex().w_full().min_w(px(0.0)).h(px(116.0)).px_6().py_3().gap_4().items_center()
                    .border_b_1().border_color(cx.theme().border)
                    .child(div().v_flex().flex_1().min_w(px(0.0)).gap_1()
                        .child(div().truncate().child(bufonidae_rclone::escape_control(&summary(&job))))
                        .child(div().text_sm().child(state_label(&job.state)))
                        .child(div().text_sm().truncate().text_color(cx.theme().muted_foreground)
                            .child(bufonidae_rclone::escape_control(job.error.as_deref().unwrap_or("")))))
                    .child(button(gpui::SharedString::from(format!("details-{}", job.id))).label("Details")
                        .on_click(cx.listener(move |this, _, _, cx| { this.transfer_details = Some(detail_id.clone()); cx.notify(); })))
                    .children(action)
            }).collect::<Vec<_>>()
        })).w_full().flex_1().min_h(px(0.0));
        div()
            .v_flex()
            .size_full()
            .min_w(px(0.0))
            .child(header)
            .children(error)
            .children(refresh_error)
            .children(details)
            .children(empty)
            .child(list)
    }
}
