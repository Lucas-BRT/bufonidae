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
use crate::backend::{AccountHealth, Backend, HealthStatus, Snapshot, StartupError};
use crate::browse::{BrowseEvent, BrowseForm};
use crate::download::{DownloadEvent, DownloadForm};
use crate::format;
use crate::remove_account::{RemoveEvent, RemoveForm};
use crate::settings::SettingsView;
use crate::store::{StoreEvent, StoreForm};
use crate::theme;

/// The dialog on top of the window, if any. One at a time.
enum Modal {
    AddAccount(Entity<AddAccountForm>),
    Store(Entity<StoreForm>),
    Browse(Entity<BrowseForm>),
    Download(Entity<DownloadForm>),
    Remove(Entity<RemoveForm>),
}

/// What the Accounts screen knows about each account's state.
pub(crate) enum Health {
    NotChecked,
    Checking,
    Ready(Vec<AccountHealth>),
    Failed(String),
}

impl Health {
    pub(crate) fn of(&self, account_id: &str) -> Option<&HealthStatus> {
        match self {
            Self::Ready(all) => all.iter().find(|h| h.id == account_id).map(|h| &h.status),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Files,
    Accounts,
    Settings,
}

enum Data {
    Loading,
    Ready(Snapshot),
    Failed(String),
}

pub struct BufonidaeApp {
    backend: Result<Arc<Backend>, StartupError>,
    screen: Screen,
    /// The folder open on the Files screen (`""` is the top level).
    pub(crate) folder: String,
    data: Data,
    modal: Option<Modal>,
    pub(crate) health: Health,
    /// Absent when the app could not start.
    settings: Option<Entity<SettingsView>>,
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
            folder: String::new(),
            data: Data::Loading,
            modal: None,
            health: Health::NotChecked,
            settings: None,
        };
        if let Ok(backend) = &app.backend {
            let backend = Arc::clone(backend);
            app.settings = Some(cx.new(|cx| SettingsView::new(backend, window, cx)));
        }
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

    /// Asks every account for its space. Signs in to each, so it runs when the
    /// user opens the Accounts screen or presses Check, not on every start.
    pub(crate) fn check_health(&mut self, cx: &mut Context<Self>) {
        let Ok(backend) = &self.backend else {
            return;
        };
        if matches!(self.health, Health::Checking) {
            return;
        }
        self.health = Health::Checking;
        cx.notify();
        let job = backend.check_health();
        cx.spawn(async move |this, cx| {
            let result = match job.await {
                Ok(result) => result,
                Err(join_error) => Err(join_error.to_string()),
            };
            this.update(cx, |this, cx| {
                this.health = match result {
                    Ok(all) => Health::Ready(all),
                    Err(message) => Health::Failed(message),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Opens the sign-in-again dialog for an account.
    pub(crate) fn open_reauth(
        &mut self,
        label: String,
        email: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Ok(backend) = &self.backend else {
            return;
        };
        if self.modal.is_some() {
            return;
        }
        let backend = Arc::clone(backend);
        let form = cx.new(|cx| AddAccountForm::reauth(backend, label, email, window, cx));
        cx.subscribe(&form, |this, _form, event: &FormEvent, cx| {
            this.modal = None;
            if matches!(event, FormEvent::Added) {
                // A new password can change what the account reports.
                this.reload(cx);
                this.check_health(cx);
            }
            cx.notify();
        })
        .detach();
        self.modal = Some(Modal::AddAccount(form));
        cx.notify();
    }

    /// Opens the remove-account confirmation.
    pub(crate) fn open_remove(&mut self, account_id: &str, cx: &mut Context<Self>) {
        let Ok(backend) = &self.backend else {
            return;
        };
        let (Some(snapshot), None) = (self.snapshot(), &self.modal) else {
            return;
        };
        let Some(account) = snapshot.accounts.iter().find(|a| a.id == account_id) else {
            return;
        };
        let (files, _) = snapshot.usage(&account.id);
        let form = cx.new(|_| {
            RemoveForm::new(
                Arc::clone(backend),
                account.label.clone(),
                account.email.clone(),
                files,
                snapshot.only_on(&account.id),
            )
        });
        cx.subscribe(&form, |this, _form, event: &RemoveEvent, cx| {
            this.modal = None;
            if matches!(event, RemoveEvent::Removed) {
                this.reload(cx);
                this.health = Health::NotChecked;
            }
            cx.notify();
        })
        .detach();
        self.modal = Some(Modal::Remove(form));
        cx.notify();
    }

    /// Shows `folder` on the Files screen.
    pub(crate) fn open_folder(&mut self, folder: String, cx: &mut Context<Self>) {
        self.folder = folder;
        cx.notify();
    }

    /// Asks for a destination folder, then downloads `path` into it.
    pub(crate) fn start_download(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.modal.is_some() || self.backend.is_err() {
            return;
        }
        // Only folders are wanted here, which is what the system dialog offers.
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Download here".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(mut dirs))) = picked.await
                && let Some(out) = dirs.pop()
            {
                this.update_in(cx, |this, _window, cx| this.open_download(path, out, cx))
                    .ok();
            }
        })
        .detach();
    }

    fn open_download(&mut self, path: String, out: PathBuf, cx: &mut Context<Self>) {
        let Ok(backend) = &self.backend else {
            return;
        };
        if self.modal.is_some() {
            return;
        }
        let backend = Arc::clone(backend);
        let form = cx.new(|cx| DownloadForm::new(&backend, path, out, cx));
        cx.subscribe(&form, |this, _form, _event: &DownloadEvent, cx| {
            this.modal = None;
            cx.notify();
        })
        .detach();
        self.modal = Some(Modal::Download(form));
        cx.notify();
    }

    /// Opens the file browser, where files and folders can be mixed.
    pub(crate) fn open_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(backend) = &self.backend else {
            return;
        };
        if self.modal.is_some() {
            return;
        }
        let backend = Arc::clone(backend);
        let form = cx.new(|cx| BrowseForm::new(backend, cx));
        cx.subscribe_in(
            &form,
            window,
            |this, _form, event: &BrowseEvent, window, cx| {
                this.modal = None;
                if let BrowseEvent::Chosen(paths) = event {
                    this.open_store(paths.clone(), window, cx);
                }
                cx.notify();
            },
        )
        .detach();
        self.modal = Some(Modal::Browse(form));
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
                    Ok(snapshot) => {
                        // The open folder may be gone after a reload.
                        if !crate::tree::exists(&snapshot.files, &this.folder) {
                            this.folder.clear();
                        }
                        Data::Ready(snapshot)
                    }
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
                    // The first visit to Accounts asks each account for its space.
                    if target == Screen::Accounts && matches!(this.health, Health::NotChecked) {
                        this.check_health(cx);
                    }
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
            .child(
                SidebarGroup::new("App").child(SidebarMenu::new().child(item(
                    "Settings",
                    IconName::Settings,
                    Screen::Settings,
                    cx,
                ))),
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
                Screen::Settings => match &self.settings {
                    Some(settings) => settings.clone().into_any_element(),
                    None => div().into_any_element(),
                },
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
            Modal::Browse(form) => form.clone().into_any_element(),
            Modal::Download(form) => form.clone().into_any_element(),
            Modal::Remove(form) => form.clone().into_any_element(),
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
                        Some(Modal::Browse(form)) => form.update(cx, |f, cx| f.cancel(cx)),
                        Some(Modal::Download(form)) => form.update(cx, |f, cx| f.close(cx)),
                        Some(Modal::Remove(form)) => form.update(cx, |f, cx| f.cancel(cx)),
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
