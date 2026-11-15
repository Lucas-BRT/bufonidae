//! The "Store files" modal: what is about to be stored, where, how many
//! copies, live progress and an honest result.

use std::path::PathBuf;
use std::sync::Arc;

use bufonidae_core::config;
use bufonidae_core::put::{Progress as PutProgress, Report};
use gpui::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    Styled as _, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, StyledExt as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    progress::Progress,
};

use crate::backend::{Backend, Inspection, StoreJob, StoreOutcome};
use crate::format;

/// How many problem lines the result lists before summarising the rest.
const MAX_LISTED: usize = 5;

pub enum StoreEvent {
    /// The dialog closed. `changed` is true when something may have been
    /// stored, so the screens should reload.
    Closed { changed: bool },
}

#[derive(Default)]
struct Counters {
    copies_total: usize,
    copies_done: usize,
    bytes_total: u64,
    bytes_done: u64,
    current: String,
    /// The user asked to stop; waiting for the job to wind down.
    cancelling: bool,
}

enum Phase {
    Inspecting,
    Ready,
    Running(Counters),
    Finished(Report),
    /// Stopped by the user after the given amount of work.
    Cancelled(Counters),
    /// Failed after some uploads may have happened: nothing to retry safely.
    Failed(String),
}

pub struct StoreForm {
    backend: Arc<Backend>,
    paths: Vec<PathBuf>,
    dest: Entity<InputState>,
    inspection: Option<Inspection>,
    phase: Phase,
    /// The user ticked "store it anyway" for a large or risky drop.
    confirmed: bool,
    /// Asks the running job to stop.
    cancel_job: Option<tokio::sync::oneshot::Sender<()>>,
    /// A problem found before anything was uploaded; the form stays editable.
    error: Option<String>,
}

impl EventEmitter<StoreEvent> for StoreForm {}

impl StoreForm {
    pub fn new(
        backend: Arc<Backend>,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let dest =
            cx.new(|cx| InputState::new(window, cx).placeholder("Top level of your accounts"));
        cx.subscribe_in(&dest, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.start(window, cx);
            }
            // The copy count depends on the folder typed here.
            cx.notify();
        })
        .detach();
        dest.update(cx, |state, cx| state.focus(window, cx));

        let job = backend.inspect(paths.clone());
        cx.spawn(async move |this, cx| {
            let result = match job.await {
                Ok(result) => result,
                Err(join_error) => Err(join_error.to_string()),
            };
            this.update(cx, |this, cx| {
                match result {
                    Ok(inspection) => {
                        this.inspection = Some(inspection);
                        this.phase = Phase::Ready;
                    }
                    Err(message) => this.phase = Phase::Failed(message),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();

        Self {
            backend,
            paths,
            dest,
            inspection: None,
            phase: Phase::Inspecting,
            confirmed: false,
            cancel_job: None,
            error: None,
        }
    }

    /// Copies wanted for the folder currently typed, and whether the accounts
    /// can provide them.
    fn plan(&self, cx: &Context<Self>) -> Option<(u8, bool)> {
        let inspection = self.inspection.as_ref()?;
        let dest = self.dest.read(cx).value();
        let copies = inspection.config.copies_for(dest.trim().trim_matches('/'));
        Some((copies, inspection.accounts >= usize::from(copies)))
    }

    /// Large drops (over the limit in Settings) and anything that includes
    /// the home folder need an explicit "store it anyway".
    fn needs_confirmation(&self) -> bool {
        self.inspection
            .as_ref()
            .is_some_and(|i| i.includes_home || i.bytes > i.config.large_upload_bytes())
    }

    fn can_start(&self, cx: &Context<Self>) -> bool {
        matches!(self.phase, Phase::Ready)
            && self
                .inspection
                .as_ref()
                .is_some_and(|i| i.files > 0 && i.accounts > 0)
            && self.plan(cx).is_some_and(|(_, enough)| enough)
            && (!self.needs_confirmation() || self.confirmed)
    }

    /// Asks the running upload to stop. rclone is shut down, which interrupts
    /// the transfer in flight; copies already stored stay stored.
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

    /// Closes the dialog, except while the upload runs.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        match self.phase {
            Phase::Running(_) => {}
            Phase::Finished(_) | Phase::Cancelled(_) => {
                cx.emit(StoreEvent::Closed { changed: true });
            }
            _ => cx.emit(StoreEvent::Closed { changed: false }),
        }
    }

    fn start(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_start(cx) {
            return;
        }
        let dest = self.dest.read(cx).value().trim().to_owned();
        self.error = None;
        self.phase = Phase::Running(Counters::default());
        cx.notify();

        let StoreJob {
            mut events,
            cancel,
            result,
        } = self.backend.store(self.paths.clone(), dest);
        self.cancel_job = Some(cancel);
        cx.spawn(async move |this, cx| {
            let mut started = false;
            while let Some(event) = events.recv().await {
                started = true;
                this.update(cx, |this, cx| {
                    if let Phase::Running(c) = &mut this.phase {
                        match event {
                            PutProgress::Planned { copies, bytes } => {
                                c.copies_total = copies;
                                c.bytes_total = bytes;
                            }
                            PutProgress::Copied { rel, size, .. } => {
                                c.copies_done += 1;
                                c.bytes_done += size;
                                c.current = rel;
                            }
                            PutProgress::Failed { rel, .. } => {
                                c.copies_done += 1;
                                c.current = rel;
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
                this.phase = match (result, started) {
                    (Ok(StoreOutcome::Done(report)), _) => Phase::Finished(report),
                    (Ok(StoreOutcome::Cancelled), _) => {
                        let counters = match std::mem::replace(&mut this.phase, Phase::Inspecting) {
                            Phase::Running(c) => c,
                            _ => Counters::default(),
                        };
                        Phase::Cancelled(counters)
                    }
                    // Failed before anything was planned: let the user fix it.
                    (Err(message), false) => {
                        this.error = Some(message);
                        Phase::Ready
                    }
                    (Err(message), true) => Phase::Failed(message),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn message_line(icon: IconName, text: String, color: gpui::Hsla) -> impl IntoElement {
        div()
            .h_flex()
            .items_start()
            .gap_2()
            .text_sm()
            .text_color(color)
            .child(Icon::new(icon).size_4().flex_none())
            .child(div().flex_1().child(text))
    }

    fn summary_of_paths(&self) -> String {
        let names: Vec<String> = self
            .paths
            .iter()
            .map(|p| {
                p.file_name().map_or_else(
                    || p.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                )
            })
            .collect();
        match names.len() {
            0 => String::new(),
            1..=3 => names.join(", "),
            n => format!("{}, {} and {} more", names[0], names[1], n - 2),
        }
    }
}

impl Render for StoreForm {
    #[allow(clippy::too_many_lines)]
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

        match &self.phase {
            Phase::Inspecting => card()
                .child(title("Store files"))
                .child(
                    div()
                        .text_color(muted)
                        .child("Looking at what you dropped…"),
                )
                .into_any_element(),

            Phase::Failed(message) => card()
                .child(title("Could not store the files"))
                .child(Self::message_line(
                    IconName::TriangleAlert,
                    message.clone(),
                    danger,
                ))
                .child(
                    div().h_flex().justify_end().child(
                        Button::new("close")
                            .primary()
                            .label("Close")
                            .on_click(cx.listener(|this, _, _, cx| {
                                // Uploads may have started: refresh the screens.
                                cx.emit(StoreEvent::Closed { changed: true });
                                let _ = this;
                            })),
                    ),
                )
                .into_any_element(),

            Phase::Running(c) => {
                #[allow(clippy::cast_precision_loss)]
                let percent = if c.bytes_total > 0 {
                    (c.bytes_done as f32 / c.bytes_total as f32 * 100.0).min(100.0)
                } else if c.copies_total > 0 {
                    c.copies_done as f32 / c.copies_total as f32 * 100.0
                } else {
                    0.0
                };
                card()
                    .child(title(if c.cancelling {
                        "Stopping…"
                    } else {
                        "Storing files"
                    }))
                    .child(Progress::new().value(percent))
                    .child(div().text_color(muted).child(format!(
                        "{} of {} copies · {} of {}",
                        c.copies_done,
                        c.copies_total,
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
                    .child(div().text_sm().text_color(muted).child(
                        "Cancel stops the transfer in progress. Copies already stored stay \
                         stored.",
                    ))
                    .child(
                        div().h_flex().justify_end().child(
                            Button::new("stop")
                                .label(if c.cancelling {
                                    "Stopping…"
                                } else {
                                    "Cancel"
                                })
                                .disabled(c.cancelling)
                                .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                        ),
                    )
                    .into_any_element()
            }

            Phase::Cancelled(c) => card()
                .child(title("Cancelled"))
                .child(
                    div()
                        .v_flex()
                        .gap_2()
                        .child(Self::message_line(
                            IconName::Info,
                            format!(
                                "{} of {} copies were processed before stopping.",
                                c.copies_done, c.copies_total
                            ),
                            muted,
                        ))
                        .child(Self::message_line(
                            IconName::TriangleAlert,
                            "The file being sent when you cancelled may be incomplete. \
                             Run `bufonidae reconcile` to check it."
                                .to_owned(),
                            muted,
                        )),
                )
                .child(
                    div().h_flex().justify_end().child(
                        Button::new("done")
                            .primary()
                            .label("Done")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    ),
                )
                .into_any_element(),

            Phase::Finished(report) => {
                let complete = report.is_complete();
                let stored = report.uploaded.len();
                let mut lines: Vec<gpui::AnyElement> = Vec::new();
                let mut push = |icon, text: String, color| {
                    lines.push(Self::message_line(icon, text, color).into_any_element());
                };
                push(
                    if complete {
                        IconName::CircleCheck
                    } else {
                        IconName::TriangleAlert
                    },
                    format!(
                        "{stored} cop{} stored",
                        if stored == 1 { "y" } else { "ies" }
                    ),
                    if complete { success } else { muted },
                );
                if !report.already_there.is_empty() {
                    push(
                        IconName::Info,
                        format!(
                            "{} already stored, left as they were",
                            format::files(report.already_there.len())
                        ),
                        muted,
                    );
                }
                if !report.skipped.is_empty() {
                    push(
                        IconName::Info,
                        format!(
                            "{} skipped (links or unsupported names)",
                            report.skipped.len()
                        ),
                        muted,
                    );
                }
                for rel in report.no_room.iter().take(MAX_LISTED) {
                    push(
                        IconName::TriangleAlert,
                        format!(
                            "{rel}: not enough accounts or space for every copy, so it was not stored"
                        ),
                        danger,
                    );
                }
                for (rel, reason) in report.failed.iter().take(MAX_LISTED) {
                    push(IconName::CircleX, format!("{rel}: {reason}"), danger);
                }
                let more = (report.no_room.len().saturating_sub(MAX_LISTED))
                    + (report.failed.len().saturating_sub(MAX_LISTED));
                if more > 0 {
                    push(IconName::Info, format!("and {more} more problems"), muted);
                }
                card()
                    .child(title(if complete {
                        "Done"
                    } else {
                        "Stored with problems"
                    }))
                    .child(div().v_flex().gap_2().children(lines))
                    .child(
                        div().h_flex().justify_end().child(
                            Button::new("done")
                                .primary()
                                .label("Done")
                                .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                        ),
                    )
                    .into_any_element()
            }

            Phase::Ready => {
                // `Ready` is only entered with an inspection in hand.
                let Some(inspection) = self.inspection.clone() else {
                    return card().child(title("Store files")).into_any_element();
                };
                let (copies, enough) = self.plan(cx).unwrap_or((1, true));
                let mut notes: Vec<gpui::AnyElement> = Vec::new();
                if inspection.accounts == 0 {
                    notes.push(
                        Self::message_line(
                            IconName::TriangleAlert,
                            "Connect a Mega account first.".to_owned(),
                            danger,
                        )
                        .into_any_element(),
                    );
                } else if !enough {
                    notes.push(
                        Self::message_line(
                            IconName::TriangleAlert,
                            format!(
                                "This folder keeps {copies} copies of each file, but only {} \
                                 account{} connected. Connect more accounts or lower the \
                                 redundancy with `bufonidae redundancy`.",
                                inspection.accounts,
                                if inspection.accounts == 1 {
                                    " is"
                                } else {
                                    "s are"
                                }
                            ),
                            danger,
                        )
                        .into_any_element(),
                    );
                }
                if inspection.files == 0 {
                    notes.push(
                        Self::message_line(
                            IconName::Info,
                            "There are no files to store in what you dropped.".to_owned(),
                            muted,
                        )
                        .into_any_element(),
                    );
                }
                if let Some(message) = &self.error {
                    notes.push(
                        Self::message_line(IconName::TriangleAlert, message.clone(), danger)
                            .into_any_element(),
                    );
                }
                if self.needs_confirmation() {
                    let text = if inspection.includes_home {
                        "This includes your whole home folder, which is almost never what \
                         you want to store."
                            .to_owned()
                    } else {
                        format!(
                            "This is {}, over the {} limit set in Settings.",
                            format::bytes(inspection.bytes),
                            format::bytes(inspection.config.large_upload_bytes())
                        )
                    };
                    notes.push(
                        div()
                            .v_flex()
                            .gap_2()
                            .child(Self::message_line(IconName::TriangleAlert, text, danger))
                            .child(
                                Checkbox::new("confirm-large")
                                    .label("I understand, store it anyway")
                                    .checked(self.confirmed)
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.confirmed = *checked;
                                        cx.notify();
                                    })),
                            )
                            .into_any_element(),
                    );
                }
                let can_start = self.can_start(cx);

                card()
                    .child(title("Store files"))
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .truncate()
                                    .font_medium()
                                    .child(self.summary_of_paths()),
                            )
                            .children((self.paths.len() == 1).then(|| {
                                div()
                                    .truncate()
                                    .text_sm()
                                    .text_color(muted)
                                    .child(self.paths[0].display().to_string())
                            }))
                            .child(div().text_color(muted).child(format!(
                                "{} · {}{}",
                                format::files(inspection.files),
                                format::bytes(inspection.bytes),
                                if inspection.skipped > 0 {
                                    format!(" · {} skipped", inspection.skipped)
                                } else {
                                    String::new()
                                }
                            ))),
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap_1p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_medium()
                                    .text_color(muted)
                                    .child("Destination folder"),
                            )
                            .child(Input::new(&self.dest)),
                    )
                    .child(div().text_color(muted).child(format!(
                        "Each file is kept on {} account{} ({}).",
                        copies,
                        if copies == 1 { "" } else { "s" },
                        config::describe(copies),
                    )))
                    .children(notes)
                    .child(
                        div()
                            .h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("cancel")
                                    .label("Cancel")
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                            )
                            .child(
                                Button::new("store")
                                    .primary()
                                    .label("Store")
                                    .disabled(!can_start)
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.start(window, cx)),
                                    ),
                            ),
                    )
                    .into_any_element()
            }
        }
    }
}
