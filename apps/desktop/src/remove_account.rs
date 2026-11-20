//! The "Remove account" confirmation: says what removing does and does not do.

use crate::interaction::{ControlFeedback as _, button};

use std::sync::Arc;

use gpui::{
    Context, EventEmitter, IntoElement, ParentElement as _, Render, Styled as _, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, StyledExt as _, button::ButtonVariants as _,
};

use crate::backend::Backend;

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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, danger) = (theme.muted_foreground, theme.danger);
        let removing = matches!(self.phase, Phase::Removing);
        let line = |icon: IconName, text: String, color: gpui::Hsla| {
            div()
                .h_flex()
                .w_full()
                .items_start()
                .gap_2()
                .text_color(color)
                .child(Icon::new(icon).size_4().flex_none())
                .child(div().flex_1().min_w(px(0.0)).child(text))
        };

        let mut body = div().v_flex().gap_2().child(line(
            IconName::Info,
            "Disconnects this account and forgets its password. Mega files stay in place."
                .to_owned(),
            muted,
        ));
        if self.only_here > 0 {
            body = body.child(line(
                IconName::TriangleAlert,
                format!(
                    "Files with no other copy ({}) become unavailable in Bufonidae. \
                     Reconnect this account and run Check to regain access.",
                    self.only_here,
                ),
                danger,
            ));
        } else if self.files > 0 {
            body = body.child(line(
                IconName::Info,
                "All files on this account have another copy in Bufonidae.".to_owned(),
                muted,
            ));
        }
        if let Phase::Failed(message) = &self.phase {
            body = body.child(line(IconName::CircleX, message.clone(), danger));
        }

        crate::ui::Dialog::new(560.0, window, cx)
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
                    .flex_none()
                    .flex_wrap()
                    .h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        button("keep")
                            .label("Keep it")
                            .disabled_feedback(removing)
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    )
                    .child(
                        button("remove")
                            .danger()
                            .label("Remove account")
                            .loading(removing && crate::motion::enabled(cx))
                            .disabled_feedback(removing)
                            .on_click(cx.listener(|this, _, _, cx| this.confirm(cx))),
                    ),
            )
    }
}
