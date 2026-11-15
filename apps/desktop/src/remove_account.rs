//! The "Remove account" confirmation: says what removing does and does not do.

use std::sync::Arc;

use gpui::{
    Context, EventEmitter, IntoElement, ParentElement as _, Render, Styled as _, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, StyledExt as _,
    button::{Button, ButtonVariants as _},
};

use crate::backend::Backend;
use crate::format;

pub enum RemoveEvent {
    Cancelled,
    Removed,
}

enum Phase {
    Asking,
    Removing,
    Failed(String),
}

pub struct RemoveForm {
    backend: Arc<Backend>,
    label: String,
    email: String,
    /// Files with a copy on this account, and how many have no other copy.
    files: usize,
    only_here: usize,
    phase: Phase,
}

impl EventEmitter<RemoveEvent> for RemoveForm {}

impl RemoveForm {
    pub fn new(
        backend: Arc<Backend>,
        label: String,
        email: String,
        files: usize,
        only_here: usize,
    ) -> Self {
        Self {
            backend,
            label,
            email,
            files,
            only_here,
            phase: Phase::Asking,
        }
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.phase, Phase::Removing) {
            cx.emit(RemoveEvent::Cancelled);
        }
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        if matches!(self.phase, Phase::Removing) {
            return;
        }
        self.phase = Phase::Removing;
        cx.notify();
        let job = self.backend.remove_account(self.label.clone());
        cx.spawn(async move |this, cx| {
            let result = match job.await {
                Ok(result) => result,
                Err(join_error) => Err(join_error.to_string()),
            };
            this.update(cx, |this, cx| {
                match result {
                    Ok(()) => cx.emit(RemoveEvent::Removed),
                    Err(message) => this.phase = Phase::Failed(message),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for RemoveForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, danger, border, surface) = (
            theme.muted_foreground,
            theme.danger,
            theme.border,
            theme.popover,
        );
        let removing = matches!(self.phase, Phase::Removing);
        let line = |icon: IconName, text: String, color: gpui::Hsla| {
            div()
                .h_flex()
                .items_start()
                .gap_2()
                .text_color(color)
                .child(Icon::new(icon).size_4().flex_none())
                .child(div().flex_1().child(text))
        };

        let mut body = div().v_flex().gap_2().child(line(
            IconName::Info,
            "Its password is forgotten and the app stops using it. Nothing is deleted from \
             the Mega account."
                .to_owned(),
            muted,
        ));
        if self.files > 0 {
            body = body.child(line(
                IconName::TriangleAlert,
                format!(
                    "{} have a copy here, and {} of them have no copy anywhere else, so they \
                     stop being available in Bufonidae. To get them back, connect this \
                     account again and check your files.",
                    format::files(self.files),
                    self.only_here
                ),
                danger,
            ));
        }
        if let Phase::Failed(message) = &self.phase {
            body = body.child(line(IconName::CircleX, message.clone(), danger));
        }

        div()
            .v_flex()
            .w(px(460.0))
            .gap_4()
            .p_6()
            .rounded(px(14.0))
            .bg(surface)
            .border_1()
            .border_color(border)
            .shadow_lg()
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xl()
                            .font_semibold()
                            .child(format!("Remove {}?", self.label)),
                    )
                    .child(div().text_color(muted).child(self.email.clone())),
            )
            .child(body)
            .child(
                div()
                    .h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("keep")
                            .label("Keep it")
                            .disabled(removing)
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    )
                    .child(
                        Button::new("remove")
                            .danger()
                            .label("Remove account")
                            .loading(removing)
                            .on_click(cx.listener(|this, _, _, cx| this.confirm(cx))),
                    ),
            )
    }
}
