//! The root view: sidebar plus the selected screen.

use std::sync::Arc;

use std::path::PathBuf;

use gpui::{
    AnyElement, AppContext as _, Context, Entity, ExternalPaths, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, PathPromptOptions, Render, Styled as _, Window,
    div, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, StyledExt as _,
    button::{Button, ButtonVariants as _},
    sidebar::{Sidebar, SidebarFooter, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem},
};

use crate::add_account::{AddAccountForm, FormEvent};
use crate::backend::{Backend, Snapshot, StartupError};
use crate::format;
use crate::store::{StoreEvent, StoreForm};
use crate::theme;

/// The dialog on top of the window, if any. One at a time.
enum Modal {
    AddAccount(Entity<AddAccountForm>),
    Store(Entity<StoreForm>),
}

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
    modal: Option<Modal>,
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
        self.modal = Some(Modal::AddAccount(form));
        cx.notify();
    }

    /// Opens the "Store files" dialog for `paths` (dropped or picked).
    pub(crate) fn open_store(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Ok(backend) = &self.backend else {
            return;
        };
        // A drop onto an open dialog is ignored rather than stacked.
        if self.modal.is_some() || paths.is_empty() {
            return;
        }
        let backend = Arc::clone(backend);
        let form = cx.new(|cx| StoreForm::new(backend, paths, window, cx));
        cx.subscribe(&form, |this, _form, event: &StoreEvent, cx| {
            this.modal = None;
            let StoreEvent::Closed { changed } = event;
            if *changed {
                this.reload(cx);
            }
            cx.notify();
        })
        .detach();
        self.modal = Some(Modal::Store(form));
        cx.notify();
    }

    /// Asks the system for files or folders to store.
    pub(crate) fn pick_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: true,
            prompt: Some("Store".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                this.update_in(cx, |this, window, cx| this.open_store(paths, window, cx))
                    .ok();
            }
        })
        .detach();
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

        let overlay = self.modal.as_ref().map(|modal| match modal {
            Modal::AddAccount(form) => form.clone().into_any_element(),
            Modal::Store(form) => form.clone().into_any_element(),
        });
        let overlay = overlay.map(|form| {
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
                    if event.keystroke.key != "escape" {
                        return;
                    }
                    match &this.modal {
                        Some(Modal::AddAccount(form)) => form.update(cx, |f, cx| f.cancel(cx)),
                        Some(Modal::Store(form)) => form.update(cx, |f, cx| f.cancel(cx)),
                        None => {}
                    }
                }))
                .child(form)
        });

        // Dropping files anywhere on the window opens the store dialog; while
        // dragging, the window is tinted to say it is a drop target.
        let drop_tint = cx.theme().accent;
        base.relative()
            .child(
                div()
                    .id("window-content")
                    .size_full()
                    .h_flex()
                    .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(drop_tint))
                    .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                        this.open_store(paths.paths().to_vec(), window, cx);
                    }))
                    .child(self.sidebar(cx))
                    .child(
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
