//! The Settings screen: how many copies to keep, and when to ask before a
//! large upload. Values come from the saved settings; if they cannot be read,
//! that is shown instead of pretending the defaults are what is saved.

use std::sync::Arc;

use bufonidae_core::config::{self, Config};
use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Window,
    div, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, StyledExt as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
};

use crate::backend::{Backend, RedundancyChange};
use crate::format;

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
}

impl SettingsView {
    pub fn new(backend: Arc<Backend>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let limit = cx.new(|cx| InputState::new(window, cx).placeholder("1"));
        // Enter or leaving the field saves.
        cx.subscribe(&limit, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.save_limit(cx);
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

        let job = backend.load_config();
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
                        this.config = Some(config);
                    }
                    Err(message) => this.load_error = Some(message),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();

        Self {
            backend,
            config: None,
            load_error: None,
            limit,
            limit_status: Status::Idle,
            rule_folder,
            rule_copies: 2,
            redundancy_status: Status::Idle,
        }
    }

    fn save_limit(&mut self, cx: &mut Context<Self>) {
        // Never write over settings that were not read successfully.
        if self.config.is_none() {
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
                    Ok(()) => Status::Saved,
                    Err(message) => Status::Invalid(message),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn change(&mut self, change: RedundancyChange, cx: &mut Context<Self>) {
        if self.config.is_none() {
            return;
        }
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
) -> Option<impl IntoElement + use<>> {
    match status {
        Status::Idle => None,
        Status::Saving => Some(
            div()
                .text_sm()
                .text_color(muted)
                .child("Saving…")
                .into_any_element(),
        ),
        Status::Saved => Some(
            div()
                .text_sm()
                .text_color(muted)
                .child("Saved.")
                .into_any_element(),
        ),
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
        let card = || {
            div()
                .v_flex()
                .gap_3()
                .max_w(px(560.0))
                .p_5()
                .rounded(px(12.0))
                .bg(surface)
                .border_1()
                .border_color(border)
        };
        let header = div()
            .px_6()
            .pt_6()
            .pb_4()
            .child(div().text_2xl().font_semibold().child("Settings"));

        if let Some(message) = &self.load_error {
            return div().v_flex().size_full().child(header).child(
                div().px_6().child(
                    div()
                        .h_flex()
                        .items_start()
                        .gap_2()
                        .text_color(danger)
                        .child(Icon::new(IconName::TriangleAlert).size_4().flex_none())
                        .child(div().flex_1().child(format!(
                            "The saved settings could not be read, so they are not shown or \
                             changed: {message}"
                        ))),
                ),
            );
        }
        let Some(current) = self.config.clone() else {
            return div()
                .v_flex()
                .size_full()
                .child(header)
                .child(div().px_6().text_color(muted).child("Loading…"));
        };

        // Level buttons: the chosen one is filled.
        let level_buttons =
            |id: &'static str,
             chosen: u8,
             cx: &mut Context<Self>,
             on_pick: fn(&mut Self, u8, &mut Context<Self>)| {
                div().h_flex().gap_1().children(LEVELS.iter().map(|&n| {
                    let button = Button::new((id, usize::from(n)))
                        .label(match n {
                            1 => "1 copy".to_owned(),
                            n => format!("{n} copies"),
                        })
                        .on_click(cx.listener(move |this, _, _, cx| on_pick(this, n, cx)));
                    if n == chosen {
                        button.primary()
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
                    .child(div().text_color(muted).child(config::describe(copies)))
                    .child(
                        Button::new(("drop-rule", i))
                            .ghost()
                            .compact()
                            .label("Remove")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.change(RedundancyChange::RemoveRule(name.clone()), cx);
                            })),
                    )
            });

        div().v_flex().size_full().child(header).child(
            div()
                .v_flex()
                .gap_4()
                .px_6()
                .pb_6()
                .child(
                    card()
                        .child(div().font_semibold().child("Copies of each file"))
                        .child(div().text_color(muted).child(
                            "Each copy goes on a different account, so more copies survive \
                                 losing an account but use more space. This applies to files \
                                 stored from now on; use Check to bring existing files up to it.",
                        ))
                        .child(default_picker)
                        .child(div().h_px().bg(border))
                        .child(div().font_medium().child("Folders with their own rule"))
                        .child(div().v_flex().gap_2().children(rules))
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .gap_2()
                                .child(div().flex_1().child(Input::new(&self.rule_folder)))
                                .child(rule_picker)
                                .child(Button::new("add-rule").label("Add rule").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.add_rule(cx);
                                    }),
                                )),
                        )
                        .children(status_line(&self.redundancy_status, muted, danger)),
                )
                .child(
                    card()
                        .child(div().font_semibold().child("Large uploads"))
                        .child(div().text_color(muted).child(
                            "Ask for confirmation before storing more than this in one go. \
                                 Dropping your whole home folder always asks. Set 0 to always ask.",
                        ))
                        .child(div().w(px(200.0)).child(
                            Input::new(&self.limit).suffix(div().text_color(muted).child("GB")),
                        ))
                        .children(status_line(&self.limit_status, muted, danger)),
                ),
        )
    }
}
