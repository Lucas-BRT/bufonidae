//! The root view: sidebar plus the selected screen.

use std::sync::Arc;

use gpui::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, Render, Styled as _, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, StyledExt as _,
    button::{Button, ButtonVariants as _},
    sidebar::{Sidebar, SidebarFooter, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem},
};

use crate::add_account::{AddAccountForm, FormEvent};
use crate::backend::{Backend, Snapshot, StartupError};
use crate::format;
use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Files,
    Accounts,
}

enum Data {
    Loading,
    Ready(Snapshot),
    Failed(String),
}

pub struct BufonidaeApp {
    backend: Result<Arc<Backend>, StartupError>,
    screen: Screen,
    data: Data,
    /// The open "Add account" modal, if any.
    modal: Option<Entity<AddAccountForm>>,
}

impl BufonidaeApp {
    pub fn new(
        backend: Result<Arc<Backend>, StartupError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Follow the system light/dark switch while the app is open.
        window
            .observe_window_appearance(theme::follow_system)
            .detach();
        let mut app = Self {
            backend,
            screen: Screen::Files,
            data: Data::Loading,
            modal: None,
        };
        app.reload(cx);
        app
    }

    pub(crate) fn snapshot(&self) -> Option<&Snapshot> {
        match &self.data {
            Data::Ready(snapshot) => Some(snapshot),
            _ => None,
        }
    }

    /// Opens the "Add account" modal.
    pub(crate) fn open_add_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(backend) = &self.backend else {
            return;
        };
        if self.modal.is_some() {
            return;
        }
        let backend = Arc::clone(backend);
        let form = cx.new(|cx| AddAccountForm::new(backend, window, cx));
        cx.subscribe(&form, |this, _form, event: &FormEvent, cx| {
            this.modal = None;
            if matches!(event, FormEvent::Added) {
                this.reload(cx);
            }
            cx.notify();
        })
        .detach();
        self.modal = Some(form);
        cx.notify();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Ok(backend) = &self.backend else {
            return;
        };
        let load = backend.load();
        cx.spawn(async move |this, cx| {
            let result = match load.await {
                Ok(result) => result,
                Err(join_error) => Err(join_error.to_string()),
            };
            this.update(cx, |this, cx| {
                this.data = match result {
                    Ok(snapshot) => Data::Ready(snapshot),
                    Err(message) => Data::Failed(message),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let item = |label: &'static str, icon: IconName, target: Screen, cx: &mut Context<Self>| {
            SidebarMenuItem::new(label)
                .icon(icon)
                .active(self.screen == target)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.screen = target;
                    cx.notify();
                }))
        };
        let summary = match &self.data {
            Data::Ready(s) => format!(
                "{} · {}",
                format::files(s.files.len()),
                format::bytes(s.total_bytes())
            ),
            _ => String::new(),
        };
        Sidebar::left()
            .header(SidebarHeader::new().child(div().text_lg().font_semibold().child("Bufonidae")))
            .child(
                SidebarGroup::new("Library").child(
                    SidebarMenu::new()
                        .child(item("Files", IconName::Folder, Screen::Files, cx))
                        .child(item("Accounts", IconName::CircleUser, Screen::Accounts, cx)),
                ),
            )
            .footer(
                SidebarFooter::new().child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(summary),
                ),
            )
    }

    fn body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match &self.data {
            Data::Loading => centered(cx, None, "Loading…", None).into_any_element(),
            Data::Failed(message) => centered(
                cx,
                Some(IconName::TriangleAlert),
                "Could not read your library",
                Some(message.clone()),
            )
            .into_any_element(),
            Data::Ready(_) => match self.screen {
                Screen::Files => self.files_screen(window, cx).into_any_element(),
                Screen::Accounts => self.accounts_screen(window, cx).into_any_element(),
            },
        }
    }
}

impl Render for BufonidaeApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let base = div()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground);

        if let Err(error) = &self.backend {
            let (title, detail) = match error {
                StartupError::InUse => (
                    "Bufonidae is already open",
                    "Another Bufonidae process (the desktop app or a terminal command) is using \
                     your library. Close it and start the app again."
                        .to_owned(),
                ),
                StartupError::Other(message) => ("Bufonidae could not start", message.clone()),
            };
            return base
                .child(
                    centered(cx, Some(IconName::TriangleAlert), title, Some(detail)).child(
                        Button::new("quit")
                            .primary()
                            .label("Close")
                            .on_click(|_, _, cx| cx.quit()),
                    ),
                )
                .into_any_element();
        }

        let overlay = self.modal.clone().map(|form| {
            // A scrim over the whole window with the form centred on it.
            // Escape closes the form unless a sign-in is running.
            div()
                .id("modal")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(cx.theme().overlay)
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape"
                        && let Some(form) = &this.modal
                    {
                        form.update(cx, |form, cx| form.cancel(cx));
                    }
                }))
                .child(form)
        });

        base.relative()
            .child(
                div().size_full().h_flex().child(self.sidebar(cx)).child(
                    div()
                        .v_flex()
                        .flex_1()
                        .h_full()
                        .min_w(px(0.0))
                        .child(self.body(window, cx)),
                ),
            )
            .children(overlay)
            .into_any_element()
    }
}

/// A centered message: optional icon, a title and an optional muted detail.
pub fn centered(
    cx: &Context<BufonidaeApp>,
    icon: Option<IconName>,
    title: &str,
    detail: Option<String>,
) -> gpui::Div {
    let muted = cx.theme().muted_foreground;
    div()
        .v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .p_8()
        .children(icon.map(|i| Icon::new(i).size_8().text_color(muted)))
        .child(div().text_lg().font_semibold().child(title.to_owned()))
        .children(detail.map(|d| {
            div()
                .max_w(px(420.0))
                .text_center()
                .text_color(muted)
                .child(d)
        }))
}
