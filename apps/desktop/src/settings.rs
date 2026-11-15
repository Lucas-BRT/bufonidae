//! The Settings screen. For now: when the app asks before a large upload.

use std::sync::Arc;

use bufonidae_core::config::{Config, DEFAULT_LARGE_UPLOAD_BYTES};
use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Window,
    div, px,
};
use gpui_component::{
    ActiveTheme as _, IconName, StyledExt as _,
    input::{Input, InputEvent, InputState},
};

use crate::backend::Backend;
use crate::format;

enum Status {
    Idle,
    Saving,
    Saved,
    Invalid(String),
}

pub struct SettingsView {
    backend: Arc<Backend>,
    limit: Entity<InputState>,
    status: Status,
}

impl SettingsView {
    pub fn new(backend: Arc<Backend>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let current = Config::default_path()
            .ok()
            .and_then(|path| Config::load(path).ok())
            .map_or(DEFAULT_LARGE_UPLOAD_BYTES, |c| c.large_upload_bytes());
        let limit = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("1")
                .default_value(format::gb_text(current))
        });
        // Enter or leaving the field saves.
        cx.subscribe(&limit, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.save(cx);
            }
        })
        .detach();
        Self {
            backend,
            limit,
            status: Status::Idle,
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let text = self.limit.read(cx).value();
        let bytes = match format::parse_gb(&text) {
            Ok(bytes) => bytes,
            Err(message) => {
                self.status = Status::Invalid(message);
                cx.notify();
                return;
            }
        };
        self.status = Status::Saving;
        cx.notify();
        let job = self.backend.save_large_upload_bytes(bytes);
        cx.spawn(async move |this, cx| {
            let result = match job.await {
                Ok(result) => result,
                Err(join_error) => Err(join_error.to_string()),
            };
            this.update(cx, |this, cx| {
                this.status = match result {
                    Ok(()) => Status::Saved,
                    Err(message) => Status::Invalid(message),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, danger, border, surface) = (
            theme.muted_foreground,
            theme.danger,
            theme.border,
            theme.popover,
        );

        let status = match &self.status {
            Status::Idle => None,
            Status::Saving => Some(div().text_sm().text_color(muted).child("Saving…")),
            Status::Saved => Some(div().text_sm().text_color(muted).child("Saved.")),
            Status::Invalid(message) => Some(
                div()
                    .h_flex()
                    .gap_2()
                    .text_sm()
                    .text_color(danger)
                    .child(gpui_component::Icon::new(IconName::TriangleAlert).size_4())
                    .child(message.clone()),
            ),
        };

        div()
            .v_flex()
            .size_full()
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .px_6()
                    .pt_6()
                    .pb_4()
                    .child(div().text_2xl().font_semibold().child("Settings")),
            )
            .child(
                div().px_6().pb_6().child(
                    div()
                        .v_flex()
                        .gap_3()
                        .max_w(px(560.0))
                        .p_5()
                        .rounded(px(12.0))
                        .bg(surface)
                        .border_1()
                        .border_color(border)
                        .child(div().font_semibold().child("Large uploads"))
                        .child(div().text_color(muted).child(
                            "Ask for confirmation before storing more than this in one go. \
                             Dropping your whole home folder always asks. Set 0 to always ask.",
                        ))
                        .child(div().w(px(200.0)).child(
                            Input::new(&self.limit).suffix(div().text_color(muted).child("GB")),
                        ))
                        .children(status),
                ),
            )
    }
}
