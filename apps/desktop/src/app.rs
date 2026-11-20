//! The root view: sidebar plus the selected screen.

use crate::interaction::{ControlFeedback as _, Navigation, button};

use std::sync::Arc;

use std::path::PathBuf;

use gpui::{
    AnyElement, AppContext as _, Context, Entity, ExternalPaths, FocusHandle,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, PathPromptOptions,
    Render, Styled as _, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, StyledExt as _,
    button::ButtonVariants as _,
    sidebar::{Sidebar, SidebarFooter, SidebarGroup, SidebarHeader},
};

use crate::add_account::{AddAccountForm, FormEvent};
use crate::backend::{AccountHealth, Backend, HealthStatus, Snapshot, StartupError};
use crate::browse::{BrowseEvent, BrowseForm};
use crate::check::CheckState;
use crate::commands::{Command, CommandEvent, CommandsForm};
use crate::download::{DownloadEvent, DownloadForm};
use crate::format;
use crate::remove_account::{RemoveEvent, RemoveForm};
use crate::settings::SettingsView;
use crate::store::{StoreEvent, StoreForm};
use crate::{motion, theme};

/// The dialog on top of the window, if any. One at a time.
#[derive(Clone)]
enum Modal {
    Commands(Entity<CommandsForm>),
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
    Check,
    Transfers,
    Settings,
    Storage,
}

enum Data {
    Loading,
    Ready(Snapshot),
    Failed(String),
}

pub struct BufonidaeApp {
    backend: Result<Arc<Backend>, StartupError>,
    screen: Screen,
    screen_generation: usize,
    pub(crate) search: Entity<gpui_component::input::InputState>,
    pub(crate) file_sort: bufonidae_core::config::FileSort,
    pub(crate) file_filter: bufonidae_core::config::FileFilter,
    pub(crate) selected_files: std::collections::BTreeSet<String>,
    /// Keyboard focus for the Files list, its highlighted row and scroll position.
    pub(crate) root_focus: FocusHandle,
    pub(crate) files_focus: FocusHandle,
    pub(crate) active_row: Option<usize>,
    pub(crate) files_scroll: gpui::UniformListScrollHandle,
    /// Whether the selection bar was showing at the last render; its fade-in
    /// replays only when the selection goes from empty to non-empty.
    pub(crate) selection_bar_visible: bool,
    pub(crate) selection_bar_generation: usize,
    /// The folder open on the Files screen (`""` is the top level).
    pub(crate) folder: String,
    data: Data,
    modal: Option<Modal>,
    presented_modal: Option<Modal>,
    modal_generation: usize,
    modal_closing: bool,
    modal_opened: std::time::Instant,
    modal_exit_opacity: f32,
    modal_focus: FocusHandle,
    return_focus: Option<FocusHandle>,
    pub(crate) health: Health,
    pub(crate) check: CheckState,
    /// Absent when the app could not start.
    settings: Option<Entity<SettingsView>>,
    distribution: Option<Entity<crate::distribution::DistributionView>>,
    pub(crate) transfer_error: Option<String>,
    pub(crate) transfer_refresh_error: Option<String>,
    pub(crate) verification_stopping: bool,
    pub(crate) transfers_refreshing: bool,
    pub(crate) transfer_details: Option<String>,
    /// Check result groups the user folded away, by title.
    pub(crate) check_folded: std::collections::BTreeSet<String>,
    /// Focus lands in the open details panel; Escape returns it to the window.
    pub(crate) transfer_panel_focus: FocusHandle,
    pub(crate) verification_cancel: Option<tokio::sync::oneshot::Sender<()>>,
    /// Paths that arrived with the last reload, highlighted for a moment.
    pub(crate) fresh: std::collections::BTreeSet<String>,
    fresh_generation: usize,
    /// The sidebar storage meter eases between these when a check finishes.
    storage_from: f32,
    storage_to: f32,
    storage_revision: usize,
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
        let options = crate::preferences::current(cx);
        let search = cx.new(|cx| {
            gpui_component::input::InputState::new(window, cx).placeholder("Search this folder")
        });
        cx.subscribe(
            &search,
            |_, _, _: &gpui_component::input::InputEvent, cx| cx.notify(),
        )
        .detach();
        let mut app = Self {
            backend,
            screen: Screen::Files,
            screen_generation: 0,
            search,
            file_sort: options.sort,
            file_filter: options.filter,
            selected_files: std::collections::BTreeSet::new(),
            root_focus: cx.focus_handle(),
            files_focus: cx.focus_handle().tab_stop(true),
            active_row: None,
            files_scroll: gpui::UniformListScrollHandle::new(),
            selection_bar_visible: false,
            selection_bar_generation: 0,
            folder: String::new(),
            data: Data::Loading,
            modal: None,
            presented_modal: None,
            modal_generation: 0,
            modal_closing: false,
            modal_opened: std::time::Instant::now(),
            modal_exit_opacity: 1.0,
            modal_focus: cx.focus_handle().tab_stop(true),
            return_focus: None,
            health: Health::NotChecked,
            check: CheckState::Idle,
            settings: None,
            distribution: None,
            transfer_error: None,
            transfer_refresh_error: None,
            verification_cancel: None,
            fresh: std::collections::BTreeSet::new(),
            fresh_generation: 0,
            storage_from: 0.0,
            storage_to: 0.0,
            storage_revision: 0,
            verification_stopping: false,
            transfers_refreshing: false,
            transfer_details: None,
            check_folded: std::collections::BTreeSet::new(),
            transfer_panel_focus: cx.focus_handle().tab_stop(true),
        };
        if let Ok(backend) = &app.backend {
            let backend = Arc::clone(backend);
            app.distribution = Some(cx.new(|cx| {
                crate::distribution::DistributionView::new(Arc::clone(&backend), window, cx)
            }));
            if let Some(view) = &app.distribution {
                cx.subscribe(
                    view,
                    |this, _, event: &crate::distribution::DistributionEvent, cx| {
                        match event {
                            crate::distribution::DistributionEvent::Started => {
                                this.refresh_transfers(cx)
                            }
                            crate::distribution::DistributionEvent::Finished => this.reload(cx),
                            crate::distribution::DistributionEvent::Health(health) => {
                                this.health = Health::Ready(health.clone())
                            }
                        }
                        cx.notify();
                    },
                )
                .detach();
            }
            app.settings = Some(cx.new(|cx| SettingsView::new(backend, window, cx)));
        }
        cx.observe_global::<motion::Preferences>(|_, cx| cx.notify())
            .detach();
        cx.observe_global::<crate::preferences::Preferences>(|this, cx| {
            let options = crate::preferences::current(cx);
            this.file_sort = options.sort;
            this.file_filter = options.filter;
            cx.notify();
        })
        .detach();
        cx.spawn_in(window, async move |this, cx| {
            let mut previous_status = String::new();
            loop {
                gpui::Timer::after(std::time::Duration::from_millis(200)).await;
                if this
                    .update_in(cx, |this, window, cx| {
                        let Ok(backend) = &this.backend else {
                            return;
                        };
                        let Some(tray) = &backend.tray else {
                            return;
                        };
                        tray.configure(crate::preferences::current(cx));
                        let action = tray.action();
                        let status = tray.status();
                        if status != previous_status {
                            previous_status = status;
                            if let Some(settings) = &this.settings {
                                settings.update(cx, |_, cx| cx.notify());
                            }
                        }
                        if let Some(action) = action {
                            window.activate_window();
                            if this.modal.is_some() {
                                return;
                            }
                            match action {
                                crate::tray::Action::Show => {}
                                crate::tray::Action::Transfers => {
                                    this.execute_command(Command::Transfers, window, cx)
                                }
                                crate::tray::Action::Settings => {
                                    this.execute_command(Command::Settings, window, cx)
                                }
                            }
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        app.reload(cx);
        window.focus(&app.root_focus);
        cx.spawn(async move |this, cx| {
            loop {
                gpui::Timer::after(std::time::Duration::from_secs(2)).await;
                if this
                    .update(cx, |this, cx| {
                        if this.screen == Screen::Transfers
                            || this.screen == Screen::Storage
                            || this.modal.is_some()
                            || this.snapshot().is_some_and(|snapshot| {
                                snapshot
                                    .jobs
                                    .iter()
                                    .any(|job| matches!(job.state.as_str(), "queued" | "running"))
                            })
                        {
                            this.refresh_transfers(cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        app
    }

    fn execute_command(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        let target = match command {
            Command::Files | Command::Search => Some(Screen::Files),
            Command::Accounts => Some(Screen::Accounts),
            Command::Check => Some(Screen::Check),
            Command::Transfers => Some(Screen::Transfers),
            Command::Settings => Some(Screen::Settings),
            Command::Storage => Some(Screen::Storage),
            _ => None,
        };
        if let Some(target) = target {
            if self.screen != target {
                self.screen_generation += 1;
            }
            self.screen = target;
            if !matches!(command, Command::Search) {
                window.focus(&self.root_focus);
            }
        }
        match command {
            Command::Search => self.search.update(cx, |field, cx| field.focus(window, cx)),
            Command::Accounts => self.check_health(cx),
            Command::Transfers => self.refresh_transfers(cx),
            Command::Storage => {
                if let Some(view) = &self.distribution {
                    view.update(cx, |view, cx| view.refresh(cx));
                }
            }
            Command::Add => {
                if self.snapshot().is_some_and(|s| s.accounts.is_empty()) {
                    self.open_add_account(window, cx);
                } else {
                    self.open_browser(window, cx);
                }
            }
            Command::DownloadSelection => self.download_selected(window, cx),
            Command::Refresh => {
                if self.screen == Screen::Accounts {
                    self.check_health(cx);
                } else if self.screen == Screen::Transfers {
                    self.refresh_transfers(cx);
                } else if self.screen == Screen::Storage {
                    if let Some(view) = &self.distribution {
                        view.update(cx, |view, cx| view.refresh(cx));
                    }
                } else {
                    self.reload(cx);
                }
            }
            Command::Help => self.open_commands(true, window, cx),
            _ => {}
        }
        cx.notify();
    }

    fn open_commands(&mut self, help: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal.is_some() {
            return;
        }
        self.prepare_modal(window, cx);
        let form = cx.new(|cx| CommandsForm::new(help, window, cx));
        cx.subscribe_in(
            &form,
            window,
            |this, _, event: &CommandEvent, window, cx| {
                this.modal = None;
                if let CommandEvent::Chosen(command) = event {
                    this.execute_command(*command, window, cx);
                }
                cx.notify();
            },
        )
        .detach();
        self.modal_generation += 1;
        self.modal_closing = false;
        self.modal = Some(Modal::Commands(form));
        cx.notify();
    }

    fn prepare_modal(&mut self, window: &mut Window, cx: &Context<Self>) {
        if self.return_focus.is_none() {
            self.return_focus = window.focused(cx);
        }
        self.modal_opened = std::time::Instant::now();
        window.focus(&self.modal_focus);
    }

    fn sync_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal.is_some() {
            self.presented_modal = self.modal.clone();
            return;
        }
        if self.presented_modal.is_none() {
            if let Some(focus) = self.return_focus.take() {
                window.focus(&focus);
            } else if window.focused(cx).is_none() {
                window.focus(&self.root_focus);
            }
            return;
        }
        if !motion::enabled(cx) {
            self.presented_modal = None;
            self.modal_closing = false;
        } else if !self.modal_closing {
            self.modal_closing = true;
            self.modal_exit_opacity = motion::entrance_opacity(self.modal_opened.elapsed());
            let generation = self.modal_generation;
            cx.spawn(async move |this, cx| {
                gpui::Timer::after(motion::EXIT).await;
                this.update(cx, |this, cx| {
                    if this.modal_generation == generation && this.modal.is_none() {
                        this.presented_modal = None;
                        this.modal_closing = false;
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
        if !self.modal_closing
            && let Some(focus) = self.return_focus.take()
        {
            window.focus(&focus);
        }
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
        self.prepare_modal(window, cx);
        let form = cx.new(|cx| AddAccountForm::new(backend, window, cx));
        cx.subscribe(&form, |this, _form, event: &FormEvent, cx| {
            this.modal = None;
            if matches!(event, FormEvent::Added) {
                this.reload(cx);
            }
            cx.notify();
        })
        .detach();
        self.modal_generation += 1;
        self.modal_closing = false;
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
        self.prepare_modal(window, cx);
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
        self.modal_generation += 1;
        self.modal_closing = false;
        self.modal = Some(Modal::Store(form));
        cx.notify();
    }

    pub(crate) fn review_upload(
        &mut self,
        paths: Vec<PathBuf>,
        destination: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_store(paths, window, cx);
        if let Some(Modal::Store(form)) = &self.modal {
            form.update(cx, |form, cx| form.set_destination(destination, window, cx));
        }
    }

    /// Asks every account for its space. Signs in to each, so it runs when the
    /// library loads or the user explicitly refreshes storage.
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
                let visible = this.storage_to;
                this.health = match result {
                    Ok(all) => Health::Ready(all),
                    Err(message) => Health::Failed(message),
                };
                let reached = this.storage_percent();
                if (reached - visible).abs() > f32::EPSILON {
                    this.storage_from = visible;
                    this.storage_to = reached;
                    this.storage_revision += 1;
                }
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
        self.prepare_modal(window, cx);
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
        self.modal_generation += 1;
        self.modal_closing = false;
        self.modal = Some(Modal::AddAccount(form));
        cx.notify();
    }

    /// Opens the remove-account confirmation.
    pub(crate) fn open_remove(
        &mut self,
        account_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        self.prepare_modal(window, cx);
        self.modal_generation += 1;
        self.modal_closing = false;
        self.modal = Some(Modal::Remove(form));
        cx.notify();
    }

    /// Goes to the Check screen and runs a check.
    pub(crate) fn go_check(&mut self, cx: &mut Context<Self>) {
        if self.screen != Screen::Check {
            self.screen_generation += 1;
        }
        self.screen = Screen::Check;
        self.run_check(false, cx);
    }

    /// Compares the index with the accounts. `apply` writes the safe fixes to
    /// the index; without it nothing changes.
    pub(crate) fn run_check(&mut self, apply: bool, cx: &mut Context<Self>) {
        let Ok(backend) = &self.backend else {
            return;
        };
        if matches!(self.check, CheckState::Running(_)) {
            return;
        }
        self.check = CheckState::Running(if apply {
            "Applying the fixes."
        } else {
            "Comparing with your accounts."
        });
        cx.notify();
        let job = backend.check_files(apply);
        cx.spawn(async move |this, cx| {
            let result = match job.await {
                Ok(result) => result,
                Err(join_error) => Err(join_error.to_string()),
            };
            this.update(cx, |this, cx| {
                this.check = match result {
                    Ok(outcome) => CheckState::Result {
                        outcome: Box::new(outcome),
                        applied: apply,
                        repaired: None,
                    },
                    Err(message) => CheckState::Failed(message),
                };
                // Applying changes what the Files screen shows.
                if apply {
                    this.reload(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Rebuilds missing copies, then checks again to show the new state.
    pub(crate) fn run_repair(&mut self, cx: &mut Context<Self>) {
        let Ok(backend) = &self.backend else {
            return;
        };
        if matches!(self.check, CheckState::Running(_)) {
            return;
        }
        self.check = CheckState::Running("Rebuilding missing copies.");
        cx.notify();
        let repair_job = backend.repair_copies();
        let backend = Arc::clone(backend);
        cx.spawn(async move |this, cx| {
            let repaired = match repair_job.await {
                Ok(result) => result,
                Err(join_error) => Err(join_error.to_string()),
            };
            let state = match repaired {
                Err(message) => CheckState::Failed(message),
                Ok(report) => match backend.check_files(false).await {
                    Ok(Ok(outcome)) => CheckState::Result {
                        outcome: Box::new(outcome),
                        applied: false,
                        repaired: Some(Box::new(report)),
                    },
                    Ok(Err(message)) => CheckState::Failed(message),
                    Err(join_error) => CheckState::Failed(join_error.to_string()),
                },
            };
            this.update(cx, |this, cx| {
                this.check = state;
                this.reload(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn run_deep_check(&mut self, cx: &mut Context<Self>) {
        if matches!(self.check, CheckState::Running(_)) {
            return;
        }
        let Ok(backend) = &self.backend else {
            return;
        };
        let job = backend.verify_copies();
        self.verification_cancel = Some(job.cancel);
        self.verification_stopping = false;
        self.check = CheckState::Running("Downloading copies to verify content.");
        cx.notify();
        let mut events = job.events;
        cx.spawn(async move |this, cx| {
            while let Some(label) = events.recv().await {
                if this
                    .update(cx, |this, cx| {
                        if !this.verification_stopping {
                            this.check = CheckState::Running(label);
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
            }
            let state = match job.result.await {
                Ok(Ok(Some(report))) => CheckState::Verified(Box::new(report)),
                Ok(Ok(None)) => CheckState::VerificationCancelled,
                Ok(Err(message)) => CheckState::Failed(message),
                Err(error) => CheckState::Failed(error.to_string()),
            };
            this.update(cx, |this, cx| {
                this.verification_cancel = None;
                this.verification_stopping = false;
                this.check = state;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Shows `folder` on the Files screen.
    pub(crate) fn open_folder(&mut self, folder: String, cx: &mut Context<Self>) {
        self.folder = folder;
        self.active_row = None;
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
                this.update_in(cx, |this, window, cx| {
                    this.open_download(path, out, window, cx)
                })
                .ok();
            }
        })
        .detach();
    }

    pub(crate) fn download_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_files.is_empty() || self.modal.is_some() {
            return;
        }
        let paths = self.selected_files.iter().cloned().collect::<Vec<_>>();
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Download selected here".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(mut dirs))) = picked.await
                && let Some(out) = dirs.pop()
            {
                this.update_in(cx, |this, window, cx| {
                    this.open_download_selection(paths, out, window, cx)
                })
                .ok();
            }
        })
        .detach();
    }

    pub(crate) fn open_download(
        &mut self,
        path: String,
        out: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_download_selection(vec![path], out, window, cx);
    }

    pub(crate) fn open_download_selection(
        &mut self,
        paths: Vec<String>,
        out: PathBuf,
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
        self.prepare_modal(window, cx);
        let form = cx.new(|cx| DownloadForm::new_selection(&backend, paths, out, cx));
        cx.subscribe(&form, |this, _form, _event: &DownloadEvent, cx| {
            this.modal = None;
            this.reload(cx);
            cx.notify();
        })
        .detach();
        self.modal_generation += 1;
        self.modal_closing = false;
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
        self.prepare_modal(window, cx);
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
        self.modal_generation += 1;
        self.modal_closing = false;
        self.modal = Some(Modal::Browse(form));
        cx.notify();
    }

    pub(crate) fn refresh_transfers(&mut self, cx: &mut Context<Self>) {
        if self.transfers_refreshing {
            return;
        }
        let Ok(backend) = &self.backend else {
            return;
        };
        let load = backend.load_jobs();
        self.transfers_refreshing = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = match load.await { Ok(result) => result, Err(error) => Err(error.to_string()) };
            this.update(cx, |this, cx| {
                this.transfers_refreshing = false;
                match result {
                    Ok(jobs) => {
                        if let Data::Ready(snapshot) = &mut this.data { snapshot.jobs = jobs; }
                        this.transfer_refresh_error = None;
                    }
                    Err(error) => this.transfer_refresh_error = Some(format!("Could not refresh history. The previous results are still shown. Retry with Refresh. {error}")),
                }
                cx.notify();
            }).ok();
        }).detach();
    }

    /// Marks what a reload added, and eases the sidebar size to its new value.
    /// The first load has nothing to compare with, so it stays still.
    fn note_changes(
        &mut self,
        before: Option<std::collections::BTreeSet<String>>,
        cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = self.snapshot() else {
            return;
        };
        let fresh: std::collections::BTreeSet<String> = before
            .map(|old| {
                snapshot
                    .files
                    .iter()
                    .filter(|f| !old.contains(&f.path))
                    // A bulk import is not worth highlighting row by row.
                    .take(50)
                    .map(|f| f.path.clone())
                    .collect()
            })
            .unwrap_or_default();
        if fresh.is_empty() {
            return;
        }
        let hold = if motion::enabled(cx) {
            motion::FLASH
        } else {
            motion::NEW_CHIP
        };
        self.fresh = fresh;
        self.fresh_generation += 1;
        let generation = self.fresh_generation;
        cx.spawn(async move |this, cx| {
            gpui::Timer::after(hold).await;
            this.update(cx, |this, cx| {
                if this.fresh_generation == generation {
                    this.fresh.clear();
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn screen_generation(&self) -> usize {
        self.screen_generation
    }

    pub(crate) fn fresh_generation(&self) -> usize {
        self.fresh_generation
    }

    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
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
                let before: Option<std::collections::BTreeSet<String>> = this
                    .snapshot()
                    .map(|s| s.files.iter().map(|f| f.path.clone()).collect());
                this.data = match result {
                    Ok(snapshot) => {
                        this.selected_files.retain(|path| {
                            snapshot.files.iter().any(|file| {
                                file.path == *path || file.path.starts_with(&format!("{path}/"))
                            })
                        });
                        // The open folder may be gone after a reload.
                        if !crate::tree::exists(&snapshot.files, &this.folder) {
                            this.folder.clear();
                        }
                        Data::Ready(snapshot)
                    }
                    Err(message) => Data::Failed(message),
                };
                this.note_changes(before, cx);
                let needs_storage = this.snapshot().is_some_and(|snapshot| {
                    !snapshot.accounts.is_empty()
                        && match &this.health {
                            Health::NotChecked => true,
                            Health::Ready(all) => {
                                all.len() != snapshot.accounts.len()
                                    || snapshot
                                        .accounts
                                        .iter()
                                        .any(|account| this.health.of(&account.id).is_none())
                            }
                            _ => false,
                        }
                });
                if needs_storage {
                    this.check_health(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn sidebar(&self, collapsed: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let queued = self.snapshot().map_or(0, |snapshot| {
            snapshot
                .jobs
                .iter()
                .filter(|job| matches!(job.state.as_str(), "queued" | "running"))
                .count()
        });
        let pending = self.snapshot().map_or(0, |snapshot| snapshot.pending);
        let item =
            move |label: &'static str, icon: IconName, target: Screen, cx: &mut Context<Self>| {
                let count = match target {
                    Screen::Transfers => queued,
                    Screen::Check => pending,
                    _ => 0,
                };
                let noun = match target {
                    Screen::Transfers => "active transfers",
                    _ => "unfinished copies",
                };
                let tooltip = if count > 0 {
                    format!("{label} · {count} {noun}")
                } else {
                    label.to_owned()
                };
                let badge = (count > 0 && !collapsed).then(|| {
                    div()
                        .flex_1()
                        .h_flex()
                        .justify_end()
                        .child(crate::ui::count_badge(count, cx))
                });
                let item_button = button(label)
                    .tooltip(crate::preferences::hint(
                        cx,
                        &tooltip,
                        match target {
                            Screen::Files => "Ctrl+1",
                            Screen::Accounts => "Ctrl+2",
                            Screen::Check => "Ctrl+3",
                            Screen::Transfers => "Ctrl+4",
                            Screen::Settings => "Ctrl+5",
                            Screen::Storage => "",
                        },
                        true,
                    ))
                    .ghost()
                    .icon(icon)
                    .selected(self.screen == target)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.screen != target {
                            this.screen_generation += 1;
                        }
                        this.screen = target;
                        window.focus(&this.root_focus);
                        if target == Screen::Transfers {
                            this.refresh_transfers(cx);
                        }
                        // The first visit to Accounts asks each account for its space.
                        if target == Screen::Accounts && matches!(this.health, Health::NotChecked) {
                            this.check_health(cx);
                        }
                        cx.notify();
                    }));
                if collapsed {
                    item_button.w_8()
                } else {
                    item_button
                        .w_full()
                        .justify_start()
                        .label(label)
                        .children(badge)
                }
            };
        let commands = button("open-command-palette")
            .ghost()
            .icon(IconName::SquareTerminal)
            .tooltip(crate::preferences::hint(
                cx,
                "Find an action",
                "Ctrl+K",
                false,
            ))
            .on_click(cx.listener(|this, _, window, cx| this.open_commands(false, window, cx)));
        let help = button("open-help")
            .ghost()
            .icon(IconName::BookOpen)
            .tooltip(crate::preferences::hint(
                cx,
                "Help and guided tour",
                "F1",
                false,
            ))
            .on_click(cx.listener(|this, _, window, cx| this.open_commands(true, window, cx)));
        if collapsed {
            return div()
                .v_flex()
                .w(px(56.0))
                .h_full()
                .flex_none()
                .items_center()
                .gap_1()
                .py_3()
                .bg(cx.theme().sidebar)
                .border_r_1()
                .border_color(cx.theme().border)
                .child(commands.w_8())
                .child(help.w_8())
                .child(div().h_3())
                .child(item("Files", IconName::Folder, Screen::Files, cx))
                .child(item("Accounts", IconName::CircleUser, Screen::Accounts, cx))
                .child(item("Check", IconName::CircleCheck, Screen::Check, cx))
                .child(item("Transfers", IconName::Inbox, Screen::Transfers, cx))
                .child(item("Settings", IconName::Settings, Screen::Settings, cx))
                .child(div().flex_1().min_h(px(0.0)))
                .child(self.storage_indicator(true, cx))
                .into_any_element();
        }
        let header = div()
            .h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .child(div().text_lg().font_semibold().child("Bufonidae"))
            .child(div().h_flex().child(commands).child(help));
        Sidebar::left()
            .header(SidebarHeader::new().child(header))
            .child(SidebarGroup::new("Library").child(Navigation::new(vec![
                item("Files", IconName::Folder, Screen::Files, cx),
                item("Accounts", IconName::CircleUser, Screen::Accounts, cx),
                item("Check", IconName::CircleCheck, Screen::Check, cx),
                item("Transfers", IconName::Inbox, Screen::Transfers, cx),
            ])))
            .child(SidebarGroup::new("App").child(Navigation::new(vec![item(
                "Settings",
                IconName::Settings,
                Screen::Settings,
                cx,
            )])))
            .footer(SidebarFooter::new().child(self.storage_indicator(false, cx)))
            .into_any_element()
    }

    fn storage_percent(&self) -> f32 {
        crate::storage::Storage::collect(
            self.snapshot()
                .into_iter()
                .flat_map(|s| s.accounts.iter())
                .map(|a| self.health.of(&a.id)),
        )
        .percent()
    }

    fn storage_indicator(&self, collapsed: bool, cx: &mut Context<Self>) -> AnyElement {
        let storage = crate::storage::Storage::collect(
            self.snapshot()
                .into_iter()
                .flat_map(|s| s.accounts.iter())
                .map(|a| self.health.of(&a.id)),
        );
        let checking = matches!(self.health, Health::Checking);
        let open = |cx: &mut Context<Self>| {
            button("open-storage-distribution")
                .ghost()
                .selected(self.screen == Screen::Storage)
                .tooltip("Open storage distribution and preview Sync")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.execute_command(Command::Storage, window, cx)
                }))
        };
        if collapsed {
            return open(cx).icon(IconName::ChartPie).w_8().into_any_element();
        }
        if self.screen == Screen::Files
            && self
                .snapshot()
                .is_some_and(|snapshot| snapshot.accounts.is_empty() && snapshot.files.is_empty())
        {
            return open(cx)
                .w_full()
                .justify_start()
                .label("Storage")
                .into_any_element();
        }
        let line = if checking {
            "Checking…".to_owned()
        } else if storage.accounts == 0 {
            "Connect an account to see capacity.".to_owned()
        } else if matches!(self.health, Health::Failed(_)) {
            "Storage check failed. Try again.".to_owned()
        } else if storage.reported == 0 {
            "Capacity unavailable.".to_owned()
        } else if storage.reported < storage.accounts {
            format!(
                "{} / {} used · {}/{} accounts",
                format::bytes(storage.used),
                format::bytes(storage.total),
                storage.reported,
                storage.accounts
            )
        } else {
            format!(
                "{} / {} used",
                format::bytes(storage.used),
                format::bytes(storage.total)
            )
        };
        let mut panel = div().v_flex().w(px(136.0)).gap_1().child(
            div()
                .h_flex()
                .w_full()
                .justify_between()
                .child(div().font_semibold().child("Storage"))
                .children(
                    (storage.reported > 0)
                        .then(|| div().text_sm().child(format!("{:.0}%", storage.percent()))),
                ),
        );
        if storage.reported > 0 {
            panel = panel.child(div().w_full().child(motion::progress(
                self.storage_from,
                self.storage_to,
                self.storage_revision,
                cx,
            )));
        }
        panel = panel.child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .w_full()
                .child(line),
        );
        div()
            .h_flex()
            .w_full()
            .min_w(px(0.0))
            .items_center()
            .gap_1()
            .child(
                div().flex_1().min_w(px(0.0)).child(
                    open(cx)
                        .w_full()
                        .h_auto()
                        .p_2()
                        .justify_start()
                        .text_left()
                        .child(panel),
                ),
            )
            .child(
                button("refresh-sidebar-storage")
                    .flex_none()
                    .ghost()
                    .icon(IconName::Redo)
                    .tooltip(if checking {
                        "Checking storage…"
                    } else {
                        "Refresh storage"
                    })
                    .disabled_feedback(checking || storage.accounts == 0)
                    .on_click(cx.listener(|this, _, _, cx| this.check_health(cx))),
            )
            .into_any_element()
    }

    fn body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match &self.data {
            Data::Loading => motion::delayed(
                centered(cx, None, "Loading…", None).into_any_element(),
                "library-loading",
                cx,
            ),
            Data::Failed(message) => centered(
                cx,
                Some(IconName::TriangleAlert),
                "Could not read your library",
                Some(message.clone()),
            )
            .child(
                button("retry-library")
                    .label("Retry loading library")
                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
            )
            .into_any_element(),
            Data::Ready(_) => match self.screen {
                Screen::Files => self.files_screen(window, cx).into_any_element(),
                Screen::Accounts => self.accounts_screen(window, cx).into_any_element(),
                Screen::Check => self.check_screen(window, cx).into_any_element(),
                Screen::Transfers => self.transfers_screen(cx).into_any_element(),
                Screen::Storage => match &self.distribution {
                    Some(view) => view.clone().into_any_element(),
                    None => div().into_any_element(),
                },
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
                        button("quit")
                            .primary()
                            .label("Close")
                            .on_click(|_, _, cx| cx.quit()),
                    ),
                )
                .into_any_element();
        }

        self.sync_modal(window, cx);
        let overlay = self.presented_modal.as_ref().map(|modal| match modal {
            Modal::Commands(form) => form.clone().into_any_element(),
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
                .occlude()
                .track_focus(&self.modal_focus)
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if this.modal_closing {
                        cx.stop_propagation();
                        return;
                    }
                    if event.keystroke.key == "tab" {
                        // Tab traversal must not reach the obscured library.
                        for _ in 0..128 {
                            if event.keystroke.modifiers.shift {
                                window.focus_prev();
                            } else {
                                window.focus_next();
                            }
                            if this.modal_focus.contains_focused(window, cx)
                                && !this.modal_focus.is_focused(window)
                            {
                                break;
                            }
                        }
                        if !this.modal_focus.contains_focused(window, cx) {
                            window.focus(&this.modal_focus);
                        }
                        cx.stop_propagation();
                    }
                }))
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key != "escape" {
                        return;
                    }
                    match &this.modal {
                        Some(Modal::Commands(form)) => form.update(cx, |form, cx| form.close(cx)),
                        Some(Modal::AddAccount(form)) => form.update(cx, |f, cx| f.cancel(cx)),
                        Some(Modal::Store(form)) => form.update(cx, |f, cx| f.cancel(cx)),
                        Some(Modal::Browse(form)) => form.update(cx, |f, cx| f.cancel(cx)),
                        Some(Modal::Download(form)) => form.update(cx, |f, cx| f.close(cx)),
                        Some(Modal::Remove(form)) => form.update(cx, |f, cx| f.cancel(cx)),
                        None => {}
                    }
                }))
                .child(motion::scrim(
                    cx.theme().overlay,
                    (
                        if self.modal_closing {
                            "scrim-exit"
                        } else {
                            "scrim-enter"
                        },
                        self.modal_generation,
                    ),
                    self.modal_closing,
                    self.modal_exit_opacity,
                    cx,
                ))
                .child(motion::dialog(
                    form,
                    (
                        if self.modal_closing {
                            "modal-exit"
                        } else {
                            "modal-enter"
                        },
                        self.modal_generation,
                    ),
                    self.modal_closing,
                    self.modal_exit_opacity,
                    cx,
                ))
                .children(
                    self.modal_closing
                        .then(|| div().absolute().inset_0().occlude()),
                )
        });

        // Dropping files anywhere on the window opens the store dialog; while
        // dragging, the window is tinted to say it is a drop target.
        let drop_accent = cx.theme().primary;
        let drop_surface = cx.theme().popover;
        base.relative()
            .track_focus(&self.root_focus)
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                use bufonidae_core::config::ShortcutMode;
                let options = crate::preferences::current(cx);
                if this.modal.is_some() || options.shortcuts == ShortcutMode::Off {
                    return;
                }
                let key = event.keystroke.key.as_str();
                let control = event.keystroke.modifiers.control;
                let command = if control {
                    match key {
                        "1" => Some(Command::Files),
                        "2" => Some(Command::Accounts),
                        "3" => Some(Command::Check),
                        "4" => Some(Command::Transfers),
                        "5" => Some(Command::Settings),
                        _ => None,
                    }
                } else {
                    None
                };
                let command = command.or_else(|| {
                    if key == "f5" {
                        Some(Command::Refresh)
                    } else if options.shortcuts == ShortcutMode::Full {
                        match (control, key) {
                            (true, "f") => Some(Command::Search),
                            (true, "o") => Some(Command::Add),
                            (false, "f1") => Some(Command::Help),
                            _ => None,
                        }
                    } else {
                        None
                    }
                });
                if let Some(command) = command {
                    this.execute_command(command, window, cx);
                    cx.stop_propagation();
                } else if options.shortcuts == ShortcutMode::Full && control && key == "k" {
                    this.open_commands(false, window, cx);
                    cx.stop_propagation();
                } else if event.keystroke.modifiers.alt
                    && key == "up"
                    && this.screen == Screen::Files
                {
                    let parent = this
                        .folder
                        .rsplit_once('/')
                        .map(|(parent, _)| parent.to_owned())
                        .unwrap_or_default();
                    this.open_folder(parent, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .id("window-content")
                    .size_full()
                    .h_flex()
                    .relative()
                    .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                        this.open_store(paths.paths().to_vec(), window, cx);
                    }))
                    .child(self.sidebar(
                        window.viewport_size().width < px(800.0)
                            || window.viewport_size().height < px(560.0),
                        cx,
                    ))
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .h_full()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .child(
                                div()
                                    .w_full()
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .overflow_hidden()
                                    .child(motion::page(
                                        self.body(window, cx),
                                        self.screen_generation,
                                        cx,
                                    )),
                            ),
                    )
                    .child(
                        // Invisible until files are dragged over the window;
                        // it never takes clicks or drops, only shows the target.
                        div()
                            .absolute()
                            .inset_0()
                            .m_2()
                            .rounded(px(12.0))
                            .border_2()
                            .border_color(drop_accent)
                            .bg(drop_accent.opacity(0.08))
                            .flex()
                            .items_center()
                            .justify_center()
                            .opacity(0.0)
                            .drag_over::<ExternalPaths>(|style, _, _, _| style.opacity(1.0))
                            .child(
                                div()
                                    .px_4()
                                    .py_2()
                                    .rounded_full()
                                    .bg(drop_surface)
                                    .border_1()
                                    .border_color(drop_accent)
                                    .font_semibold()
                                    .child("Drop to store"),
                            ),
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
        .flex_1()
        .min_h(px(0.0))
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
