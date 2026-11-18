//! The "Add account" modal: e-mail, password, an optional 2FA code and an
//! optional name. The login is proven against Mega before anything is saved.

use crate::interaction::{ControlFeedback as _, button};

use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    Styled as _, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, StyledExt as _,
    button::ButtonVariants as _,
    input::{Input, InputEvent, InputState},
};

use crate::backend::{Backend, NewAccount, suggest_label};

/// What the form tells the app.
pub enum FormEvent {
    Cancelled,
    Added,
}

enum Phase {
    Editing,
    Submitting,
    Failed(String),
}

/// What the form is for.
#[derive(Clone)]
enum Mode {
    /// Connect a new account.
    New,
    /// Sign an existing account in again (a new password, or after a restore).
    Reauth { label: String, email: String },
}

pub struct AddAccountForm {
    backend: Arc<Backend>,
    mode: Mode,
    email: Entity<InputState>,
    password: Entity<InputState>,
    code: Entity<InputState>,
    name: Entity<InputState>,
    phase: Phase,
}

impl EventEmitter<FormEvent> for AddAccountForm {}

impl AddAccountForm {
    pub fn new(backend: Arc<Backend>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let email = cx.new(|cx| InputState::new(window, cx).placeholder("you@example.com"));
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let code = cx.new(|cx| InputState::new(window, cx).placeholder("Only if you use 2FA"));
        let name =
            cx.new(|cx| InputState::new(window, cx).placeholder("Defaults to your e-mail name"));

        // Enter in any field submits.
        for input in [&email, &password, &code, &name] {
            cx.subscribe_in(input, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.submit(window, cx);
                }
            })
            .detach();
        }
        email.update(cx, |state, cx| state.focus(window, cx));

        Self {
            backend,
            mode: Mode::New,
            email,
            password,
            code,
            name,
            phase: Phase::Editing,
        }
    }

    /// The same form, for signing `label` in again: only the password and the
    /// 2FA code are asked for.
    pub fn reauth(
        backend: Arc<Backend>,
        label: String,
        email: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut form = Self::new(backend, window, cx);
        form.mode = Mode::Reauth { label, email };
        form.password
            .update(cx, |state, cx| state.focus(window, cx));
        form
    }

    /// Closes the form, except while a sign-in runs: closing then would leave
    /// the account being added in the background without the user knowing.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.phase, Phase::Submitting) {
            cx.emit(FormEvent::Cancelled);
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.phase, Phase::Submitting) {
            return;
        }
        let email = self.email.read(cx).value().trim().to_owned();
        let password = self.password.read(cx).value().to_string();
        let code = self.code.read(cx).value().trim().to_owned();
        let name = self.name.read(cx).value().trim().to_owned();

        if let Mode::Reauth { label, .. } = self.mode.clone() {
            if password.is_empty() {
                self.phase = Phase::Failed("Enter your Mega password.".into());
                self.password
                    .update(cx, |field, cx| field.focus(window, cx));
                cx.notify();
                return;
            }
            for secret in [&self.password, &self.code] {
                secret.update(cx, |field, cx| field.set_value("", window, cx));
            }
            let job = self
                .backend
                .reauthorize(label, password, (!code.is_empty()).then_some(code));
            self.run(job, cx);
            return;
        }

        if !email.contains('@') {
            self.phase = Phase::Failed("Enter the e-mail address of your Mega account.".into());
            self.email.update(cx, |field, cx| field.focus(window, cx));
            cx.notify();
            return;
        }
        if password.is_empty() {
            self.phase = Phase::Failed("Enter your Mega password.".into());
            self.password
                .update(cx, |field, cx| field.focus(window, cx));
            cx.notify();
            return;
        }
        let request = NewAccount {
            label: if name.is_empty() {
                suggest_label(&email)
            } else {
                name
            },
            email,
            password,
            two_fa: (!code.is_empty()).then_some(code),
        };

        // The secrets now live only in `request`: take them out of the text
        // fields so they do not sit in the widgets while the sign-in runs, or
        // afterwards if it fails (the user retypes them).
        for secret in [&self.password, &self.code] {
            secret.update(cx, |field, cx| field.set_value("", window, cx));
        }

        let job = self.backend.add_account(request);
        self.run(job, cx);
    }

    /// Waits for a sign-in job and reports how it ended.
    fn run(&mut self, job: tokio::task::JoinHandle<Result<(), String>>, cx: &mut Context<Self>) {
        self.phase = Phase::Submitting;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = match job.await {
                Ok(result) => result,
                Err(join_error) => Err(join_error.to_string()),
            };
            this.update(cx, |this, cx| {
                match result {
                    Ok(()) => cx.emit(FormEvent::Added),
                    Err(message) => this.phase = Phase::Failed(message),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn field(
        &self,
        cx: &Context<Self>,
        label: &'static str,
        input: &Entity<InputState>,
        submitting: bool,
    ) -> impl IntoElement {
        div()
            .v_flex()
            .gap_1p5()
            .child(
                div()
                    .text_sm()
                    .font_medium()
                    .text_color(cx.theme().muted_foreground)
                    .child(label),
            )
            .child(Input::new(input).disabled(submitting))
    }
}

impl Render for AddAccountForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let submitting = matches!(self.phase, Phase::Submitting);
        let theme = cx.theme();
        let (muted, danger, border, surface) = (
            theme.muted_foreground,
            theme.danger,
            theme.border,
            theme.popover,
        );

        let status = match &self.phase {
            Phase::Editing => None,
            Phase::Submitting => Some(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child("Signing in to Mega. This can take a few seconds."),
            ),
            // Icon and words, not only colour: the error is never just red.
            Phase::Failed(message) => Some(
                div()
                    .h_flex()
                    .items_start()
                    .gap_2()
                    .text_sm()
                    .text_color(danger)
                    .child(Icon::new(IconName::TriangleAlert).size_4().flex_none())
                    .child(div().flex_1().child(message.clone())),
            ),
        };

        div()
            .v_flex()
            .w(px(420.0))
            .max_w(window.viewport_size().width - px(32.0))
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
                    .child(div().text_xl().font_semibold().child(match &self.mode {
                        Mode::New => "Add a Mega account".to_owned(),
                        Mode::Reauth { .. } => "Sign in again".to_owned(),
                    }))
                    .child(
                        div().text_color(muted).child(match &self.mode {
                            Mode::New => "Your password stays in the system keyring and is never \
                                      written to a file."
                                .to_owned(),
                            Mode::Reauth { label, email } => format!(
                                "{label} ({email}). Enter the password again; it replaces the one \
                             stored in the system keyring."
                            ),
                        }),
                    ),
            )
            .children(
                matches!(self.mode, Mode::New)
                    .then(|| self.field(cx, "E-mail", &self.email, submitting)),
            )
            .child(self.field(cx, "Password", &self.password, submitting))
            .child(self.field(cx, "2FA code", &self.code, submitting))
            .children(
                matches!(self.mode, Mode::New)
                    .then(|| self.field(cx, "Name", &self.name, submitting)),
            )
            .children(status)
            .child(
                div()
                    .h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        button("cancel")
                            .label("Cancel")
                            .disabled_feedback(submitting)
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    )
                    .child(
                        button("connect")
                            .primary()
                            .label(match &self.mode {
                                Mode::New => "Connect account",
                                Mode::Reauth { .. } => "Sign in",
                            })
                            .loading(submitting && crate::motion::enabled(cx))
                            .disabled_feedback(submitting)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}
