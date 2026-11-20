//! The Settings screen: how many copies to keep, and when to ask before a
//! large upload. Values come from the saved settings; if they cannot be read,
//! that is shown instead of pretending the defaults are what is saved.

mod options;

use crate::interaction::{ControlFeedback as _, button, checkbox};

use std::sync::Arc;

use bufonidae_core::config::{self, Config};
use gpui::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, StyledExt as _,
    button::ButtonVariants as _,
    input::{Input, InputEvent, InputState},
};

use crate::backend::{Backend, RedundancyChange};
use crate::{format, motion};

/// Levels offered as buttons. Higher ones exist (`bufonidae redundancy`).
const LEVELS: [u8; 3] = [1, 2, 3];

enum Status {
    Idle,
    Saving,
    Saved,
    Invalid(String),
}

pub struct SettingsView {
    backend: Arc<Backend>,
    /// `None` until the saved settings have been read.
    config: Option<Config>,
    load_error: Option<String>,
    limit: Entity<InputState>,
    limit_status: Status,
    rule_folder: Entity<InputState>,
    rule_copies: u8,
    redundancy_status: Status,
    motion_status: Status,
    feedback_revision: usize,
    desktop_draft: config::DesktopOptions,
    desktop_status: Status,
    notification_feedback: Option<(crate::ui::Tone, String)>,
    scroll: gpui::ScrollHandle,
    notification_testing: bool,
}

impl SettingsView {
    pub fn new(backend: Arc<Backend>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let limit = cx.new(|cx| InputState::new(window, cx).placeholder("1"));
        // Enter or leaving the field saves.
        cx.subscribe(&limit, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.save_limit(cx);
            } else if matches!(event, InputEvent::Change) && !this.saving() {
                this.limit_status = Status::Idle;
                cx.notify();
            }
        })
        .detach();
        let rule_folder =
            cx.new(|cx| InputState::new(window, cx).placeholder("Folder, e.g. Documents"));
        cx.subscribe(&rule_folder, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.add_rule(cx);
            }
        })
        .detach();

        let mut view = Self {
            backend,
            config: None,
            load_error: None,
            limit,
            limit_status: Status::Idle,
            rule_folder,
            rule_copies: 2,
            redundancy_status: Status::Idle,
            motion_status: Status::Idle,
            feedback_revision: 0,
            desktop_draft: config::DesktopOptions::default(),
            desktop_status: Status::Idle,
            notification_feedback: None,
            scroll: gpui::ScrollHandle::new(),
            notification_testing: false,
        };
        view.load(window, cx);
        view
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.load_error = None;
        self.config = None;
        let job = self.backend.load_config();
        cx.spawn_in(window, async move |this, cx| {
            let result = match job.await {
                Ok(result) => result,
                Err(join_error) => Err(join_error.to_string()),
            };
            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(config) => {
                        let text = format::gb_text(config.large_upload_bytes());
                        this.limit
                            .update(cx, |field, cx| field.set_value(text, window, cx));
                        cx.set_global(motion::Preferences {
                            reduced: config.reduced_motion(),
                        });
                        this.desktop_draft = config.desktop().clone();
                        crate::preferences::apply(config.desktop().clone(), cx);
                        this.config = Some(config);
                    }
                    Err(message) => this.load_error = Some(message),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();

        cx.notify();
    }

    fn saving(&self) -> bool {
        matches!(self.limit_status, Status::Saving)
            || matches!(self.redundancy_status, Status::Saving)
            || matches!(self.motion_status, Status::Saving)
            || matches!(self.desktop_status, Status::Saving)
    }

    fn save_motion(&mut self, reduced: bool, cx: &mut Context<Self>) {
        if self.config.is_none() || self.saving() {
            return;
        }
        self.feedback_revision += 1;
        self.motion_status = Status::Saving;
        cx.notify();
        let job = self.backend.save_reduced_motion(reduced);
        cx.spawn(async move |this, cx| {
            let result = match job.await {
                Ok(result) => result,
                Err(error) => Err(error.to_string()),
            };
            this.update(cx, |this, cx| {
                match result {
                    Ok(config) => {
                        cx.set_global(motion::Preferences {
                            reduced: config.reduced_motion(),
                        });
                        this.config = Some(config);
                        this.motion_status = Status::Saved;
                    }
                    Err(error) => this.motion_status = Status::Invalid(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn save_limit(&mut self, cx: &mut Context<Self>) {
        // Never write over settings that were not read successfully.
        if self.config.is_none() || self.saving() {
            return;
        }
        let text = self.limit.read(cx).value();
        let bytes = match format::parse_gb(&text) {
            Ok(bytes) => bytes,
            Err(message) => {
                self.limit_status = Status::Invalid(message);
                cx.notify();
                return;
            }
        };
        if self
            .config
            .as_ref()
            .is_some_and(|config| config.large_upload_bytes() == bytes)
        {
            return;
        }
        self.feedback_revision += 1;
        self.limit_status = Status::Saving;
        cx.notify();
        let job = self.backend.save_large_upload_bytes(bytes);
        cx.spawn(async move |this, cx| {
            let result = match job.await {
                Ok(result) => result,
                Err(join_error) => Err(join_error.to_string()),
            };
            this.update(cx, |this, cx| {
                this.limit_status = match result {
                    Ok(config) => {
                        this.config = Some(config);
                        Status::Saved
                    }
                    Err(message) => Status::Invalid(message),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn change(&mut self, change: RedundancyChange, cx: &mut Context<Self>) {
        if self.config.is_none() || self.saving() {
            return;
        }
        self.feedback_revision += 1;
        self.redundancy_status = Status::Saving;
        cx.notify();
        let job = self.backend.change_redundancy(change);
        cx.spawn(async move |this, cx| {
            let result = match job.await {
                Ok(result) => result,
                Err(join_error) => Err(join_error.to_string()),
            };
            this.update(cx, |this, cx| {
                match result {
                    Ok(config) => {
                        cx.set_global(motion::Preferences {
                            reduced: config.reduced_motion(),
                        });
                        this.config = Some(config);
                        this.redundancy_status = Status::Saved;
                    }
                    Err(message) => this.redundancy_status = Status::Invalid(message),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn add_rule(&mut self, cx: &mut Context<Self>) {
        let folder = self.rule_folder.read(cx).value().trim().to_owned();
        if folder.is_empty() {
            self.redundancy_status = Status::Invalid("Enter the folder this rule is for.".into());
            cx.notify();
            return;
        }
        let copies = self.rule_copies;
        self.change(RedundancyChange::Rule { folder, copies }, cx);
    }
}

fn status_line(
    status: &Status,
    muted: gpui::Hsla,
    danger: gpui::Hsla,
    revision: usize,
    cx: &gpui::App,
) -> Option<impl IntoElement + use<>> {
    match status {
        Status::Idle => None,
        Status::Saving => Some(motion::activity("Saving settings", cx)),
        Status::Saved => Some(motion::fade(
            div()
                .h_flex()
                .gap_2()
                .items_center()
                .text_sm()
                .text_color(muted)
                .child(Icon::new(IconName::CircleCheck).size_4())
                .child("Saved on this device."),
            ("settings-saved", revision),
            false,
            1.0,
            cx,
        )),
        Status::Invalid(message) => Some(
            div()
                .h_flex()
                .items_start()
                .gap_2()
                .text_sm()
                .text_color(danger)
                .child(Icon::new(IconName::TriangleAlert).size_4().flex_none())
                .child(div().flex_1().child(message.clone()))
                .into_any_element(),
        ),
    }
}

impl Render for SettingsView {
    #[allow(clippy::too_many_lines)]
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, danger, border, surface) = (
            theme.muted_foreground,
            theme.danger,
            theme.border,
            theme.popover,
        );
        // Use normal block flow for auto-height cards. Nested flex columns can
        // size wrapped text before its final width is known and undercount it.
        let card = |children: Vec<AnyElement>| {
            div()
                .block()
                .w_full()
                .max_w(px(560.0))
                .mb_4()
                .p_5()
                .rounded(px(12.0))
                .bg(surface)
                .border_1()
                .border_color(border)
                .children(children.into_iter().enumerate().map(|(i, child)| {
                    div()
                        .block()
                        .pt(if i == 0 { px(0.0) } else { px(12.0) })
                        .child(child)
                }))
        };
        let title = div().text_xl().font_semibold().child("Settings");

        if let Some(message) = &self.load_error {
            return div().v_flex().size_full().child(div().px_6().py_3().child(title)).child(
                div().px_6().v_flex().gap_3()
                    .child(div().h_flex().items_start().gap_2().text_color(danger)
                        .child(Icon::new(IconName::TriangleAlert).size_4().flex_none())
                        .child(div().flex_1().child(format!("The saved settings could not be read, so they are not shown or changed: {message}"))))
                    .child(button("retry-settings").label("Retry loading settings")
                        .on_click(cx.listener(|this, _, window, cx| this.load(window, cx)))),
            );
        }
        let Some(current) = self.config.clone() else {
            return div()
                .v_flex()
                .size_full()
                .child(div().px_6().py_3().child(title))
                .child(
                    div()
                        .px_6()
                        .text_color(muted)
                        .child(motion::activity("Loading settings", cx)),
                );
        };

        let saving = self.saving();
        // Level buttons: the chosen one is filled.
        let level_buttons =
            |id: &'static str,
             chosen: u8,
             cx: &mut Context<Self>,
             on_pick: fn(&mut Self, u8, &mut Context<Self>)| {
                div()
                    .h_flex()
                    .flex_none()
                    .gap_1()
                    .children(LEVELS.iter().map(|&n| {
                        let button = button((id, usize::from(n)))
                            .min_w(px(108.0))
                            .selected(n == chosen)
                            .disabled_feedback(saving)
                            .label(match n {
                                1 => "1 copy".to_owned(),
                                n => format!("{n} copies"),
                            })
                            .on_click(cx.listener(move |this, _, _, cx| on_pick(this, n, cx)));
                        if n == chosen {
                            button.primary().icon(IconName::CircleCheck)
                        } else {
                            button
                        }
                    }))
            };

        let default_picker = level_buttons(
            "default-copies",
            current.default_copies(),
            cx,
            |this, n, cx| this.change(RedundancyChange::Default(n), cx),
        );
        let rule_picker = level_buttons("rule-copies", self.rule_copies, cx, |this, n, cx| {
            this.rule_copies = n;
            cx.notify();
        });

        let rules = current
            .folders()
            .iter()
            .enumerate()
            .map(|(i, (folder, &copies))| {
                let name = folder.clone();
                div()
                    .h_flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .child(bufonidae_rclone::escape_control(folder)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_color(muted)
                            .child(config::describe(copies)),
                    )
                    .child(
                        button(("drop-rule", i))
                            .ghost()
                            .compact()
                            .label("Remove")
                            .disabled_feedback(saving)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.change(RedundancyChange::RemoveRule(name.clone()), cx);
                            })),
                    )
            })
            .collect::<Vec<_>>();

        let rule_editor = div()
            .block()
            .child(
                div()
                    .h_8()
                    .mb_2()
                    .child(Input::new(&self.rule_folder).disabled(saving)),
            )
            .child(div().mb_2().child(rule_picker))
            .child(
                div().h_flex().child(
                    button("add-rule")
                        .label("Add rule")
                        .disabled_feedback(saving)
                        .on_click(cx.listener(|this, _, _, cx| this.add_rule(cx))),
                ),
            );

        // Jump links to the top-level sections of the scrolling page.
        let mut jumps = div().h_flex().min_w(px(0.0)).flex_wrap().gap_1();
        for (ix, label) in ["Copies", "Uploads", "Motion", "Desktop"]
            .into_iter()
            .enumerate()
        {
            jumps = jumps.child(
                button(("settings-jump", ix))
                    .ghost()
                    .compact()
                    .label(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.scroll.scroll_to_top_of_item(ix);
                        cx.notify();
                    })),
            );
        }
        let header = div()
            .h_flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .gap_x_4()
            .gap_y_1()
            .px_6()
            .py_2()
            .child(title.flex_none())
            .child(jumps);
        let save_bar = self.save_bar(cx);
        div()
            .v_flex()
            .size_full()
            .child(header)
            .child(
                div()
                    .id("settings-body")
                    .track_scroll(&self.scroll)
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .px_6()
                    .pb_6()
                    .child(card(vec![
                        div()
                            .font_semibold()
                            .child("Copies of each file")
                            .into_any_element(),
                        div()
                            .text_color(muted)
                            .child(
                                "Each copy goes on a different account, so more copies survive \
                         losing an account but use more space. This applies to files \
                         stored from now on; use Check to bring existing files up to it.",
                            )
                            .into_any_element(),
                        default_picker.into_any_element(),
                        div().h_px().bg(border).into_any_element(),
                        div()
                            .font_medium()
                            .child("Folders with their own rule")
                            .into_any_element(),
                        div()
                            .block()
                            .children(rules.into_iter().map(|row| div().mb_2().child(row)))
                            .into_any_element(),
                        rule_editor.into_any_element(),
                        div()
                            .children(status_line(
                                &self.redundancy_status,
                                muted,
                                danger,
                                self.feedback_revision,
                                cx,
                            ))
                            .into_any_element(),
                    ]))
                    .child(card(vec![
                        div()
                            .font_semibold()
                            .child("Large uploads")
                            .into_any_element(),
                        div()
                            .text_color(muted)
                            .child(
                                "Ask for confirmation before storing more than this in one go. \
                         Dropping your whole home folder always asks. Set 0 to always ask.",
                            )
                            .into_any_element(),
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child("Enter a size in GB. Press Enter or leave the field to save.")
                            .into_any_element(),
                        div()
                            .w(px(200.0))
                            .h_8()
                            .child(
                                Input::new(&self.limit)
                                    .disabled(saving)
                                    .suffix(div().text_color(muted).child("GB")),
                            )
                            .into_any_element(),
                        div()
                            .children(status_line(
                                &self.limit_status,
                                muted,
                                danger,
                                self.feedback_revision,
                                cx,
                            ))
                            .into_any_element(),
                    ]))
                    .child(card(vec![
                        div()
                            .font_semibold()
                            .child("Motion and feedback")
                            .into_any_element(),
                        checkbox("reduced-motion")
                            .label("Reduce motion")
                            .checked(current.reduced_motion())
                            .disabled_feedback(saving)
                            .on_click(cx.listener(|this, reduced: &bool, _, cx| {
                                this.save_motion(*reduced, cx)
                            }))
                            .into_any_element(),
                        div()
                            .text_color(muted)
                            .child("Turn off fades, smooth progress and pulsing indicators.")
                            .into_any_element(),
                        div()
                            .children(status_line(
                                &self.motion_status,
                                muted,
                                danger,
                                self.feedback_revision,
                                cx,
                            ))
                            .into_any_element(),
                    ]))
                    .child(self.desktop_panel(cx)),
            )
            .children(save_bar)
    }
}
