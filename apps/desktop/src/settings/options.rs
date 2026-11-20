use super::*;
use crate::interaction::{button, checkbox};
use bufonidae_core::config::{DesktopOptions, FileFilter, FileSort, ShortcutMode, SoundMode};

fn section(
    title: &str,
    description: &str,
    controls: Vec<AnyElement>,
    cx: &gpui::App,
) -> AnyElement {
    div()
        .v_flex()
        .w_full()
        .gap_2()
        .py_3()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(div().font_semibold().child(title.to_owned()))
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(description.to_owned()),
        )
        .children(controls)
        .into_any_element()
}

impl SettingsView {
    fn option(
        &self,
        id: &'static str,
        label: impl Into<gpui::SharedString>,
        selected: bool,
        cx: &mut Context<Self>,
        pick: impl Fn(&mut DesktopOptions) + 'static,
    ) -> AnyElement {
        let control = button(id)
            .label(label)
            .selected(selected)
            .disabled_feedback(self.saving())
            .on_click(cx.listener(move |this, _, _, cx| {
                pick(&mut this.desktop_draft);
                this.desktop_status = Status::Idle;
                cx.notify();
            }));
        if selected {
            control.primary().into_any_element()
        } else {
            control.into_any_element()
        }
    }

    fn toggle_option(
        &self,
        id: &'static str,
        label: &'static str,
        checked: bool,
        cx: &mut Context<Self>,
        pick: fn(&mut DesktopOptions, bool),
    ) -> AnyElement {
        checkbox(id)
            .label(label)
            .checked(checked)
            .disabled_feedback(self.saving())
            .on_click(cx.listener(move |this, value: &bool, _, cx| {
                pick(&mut this.desktop_draft, *value);
                this.desktop_status = Status::Idle;
                cx.notify();
            }))
            .into_any_element()
    }

    fn save_desktop(&mut self, cx: &mut Context<Self>) {
        if self.saving() {
            return;
        }
        self.feedback_revision += 1;
        self.desktop_status = Status::Saving;
        cx.notify();
        let job = self
            .backend
            .save_desktop_options(self.desktop_draft.clone());
        cx.spawn(async move |this, cx| {
            let result = match job.await {
                Ok(result) => result,
                Err(error) => Err(error.to_string()),
            };
            this.update(cx, |this, cx| {
                match result {
                    Ok(config) => {
                        crate::preferences::apply(config.desktop().clone(), cx);
                        this.desktop_draft = config.desktop().clone();
                        this.config = Some(config);
                        this.desktop_status = Status::Saved;
                    }
                    Err(error) => this.desktop_status = Status::Invalid(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn test_notifications(&mut self, failed: bool, cx: &mut Context<Self>) {
        if self.notification_testing {
            return;
        }
        self.notification_testing = true;
        self.notification_feedback = Some((
            crate::ui::Tone::Neutral,
            "Sending test notification…".into(),
        ));
        let job = self
            .backend
            .test_notification(self.desktop_draft.clone(), failed);
        cx.spawn(async move|this,cx| {
            let message=match job.await {Ok(Ok(()))=>(crate::ui::Tone::Success,"Notification accepted by the desktop service. Check your desktop's notification settings if it is hidden.".to_owned()),Ok(Err(error))=>(crate::ui::Tone::Danger,error),Err(error)=>(crate::ui::Tone::Danger,error.to_string())};
            this.update(cx,|this,cx|{this.notification_testing=false;this.notification_feedback=Some(message);cx.notify();}).ok();
        }).detach();
        cx.notify();
    }

    pub(super) fn desktop_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.desktop_draft.clone();
        let density = div()
            .h_flex()
            .flex_wrap()
            .gap_2()
            .child(self.option(
                "density-comfortable",
                "Comfortable · 52 px rows",
                !p.compact,
                cx,
                |p| p.compact = false,
            ))
            .child(self.option(
                "density-compact",
                "Compact · 40 px rows",
                p.compact,
                cx,
                |p| p.compact = true,
            ));
        let mut sizes = div().h_flex().flex_wrap().gap_2();
        for (id, size) in [
            ("text-12", 12),
            ("text-14", 14),
            ("text-16", 16),
            ("text-18", 18),
            ("text-20", 20),
        ] {
            sizes = sizes.child(self.option(
                id,
                format!("{size} px"),
                p.text_size == size,
                cx,
                move |p| p.text_size = size,
            ));
        }
        let appearance = section(
            "Density and text",
            "Desktop preferences apply when you choose Save.",
            vec![
                density.into_any_element(),
                div()
                    .font_medium()
                    .child("Base text size")
                    .into_any_element(),
                sizes.into_any_element(),
            ],
            cx,
        );
        let mut order = div().h_flex().flex_wrap().gap_2();
        for (id, label, value) in [
            ("sort-name", "Name A–Z", FileSort::Name),
            ("sort-name-desc", "Name Z–A", FileSort::NameDescending),
            ("sort-size", "Size ascending", FileSort::Size),
            (
                "sort-size-desc",
                "Size descending",
                FileSort::SizeDescending,
            ),
        ] {
            order =
                order.child(self.option(id, label, p.sort == value, cx, move |p| p.sort = value));
        }
        let mut filters = div().h_flex().flex_wrap().gap_2();
        for (id, label, value) in [
            ("filter-all", "All entries", FileFilter::All),
            ("filter-files", "Files only", FileFilter::Files),
            ("filter-folders", "Folders only", FileFilter::Folders),
        ] {
            filters = filters
                .child(self.option(id, label, p.filter == value, cx, move |p| p.filter = value));
        }
        let browsing_controls = vec![
            order.into_any_element(),
            filters.into_any_element(),
            self.toggle_option(
                "folders-first",
                "Keep folders before files",
                p.folders_first,
                cx,
                |p, v| p.folders_first = v,
            ),
            self.toggle_option(
                "match-case",
                "Match letter case when searching",
                p.case_sensitive,
                cx,
                |p, v| p.case_sensitive = v,
            ),
        ];
        let browsing = section(
            "Search, sorting and selection",
            "Defaults for Files. Search stays in the open folder and filters preserve selection.",
            browsing_controls,
            cx,
        );
        let mut duration = div().h_flex().flex_wrap().gap_2();
        for (id, seconds) in [("notify-3", 3), ("notify-5", 5), ("notify-10", 10)] {
            duration = duration.child(self.option(
                id,
                format!("{seconds} seconds"),
                p.notification_seconds == seconds,
                cx,
                move |p| p.notification_seconds = seconds,
            ));
        }
        let mut sounds = div().h_flex().flex_wrap().gap_2();
        for (id, label, value) in [
            ("sound-off", "Silent", SoundMode::Off),
            ("sound-errors", "Errors only", SoundMode::Errors),
            ("sound-all", "All enabled events", SoundMode::All),
        ] {
            sounds = sounds
                .child(self.option(id, label, p.sound == value, cx, move |p| p.sound = value));
        }
        let notification_controls = vec![
            self.toggle_option(
                "notify-success",
                "Notify when a batch completes",
                p.notify_success,
                cx,
                |p, v| p.notify_success = v,
            ),
            self.toggle_option(
                "notify-failure",
                "Notify on failures or partial completion",
                p.notify_failure,
                cx,
                |p, v| p.notify_failure = v,
            ),
            self.toggle_option(
                "notify-cancel",
                "Notify when a transfer is cancelled",
                p.notify_cancel,
                cx,
                |p, v| p.notify_cancel = v,
            ),
            self.toggle_option(
                "notify-names",
                "Include paths in desktop notifications",
                p.notification_names,
                cx,
                |p, v| p.notification_names = v,
            ),
            div()
                .text_sm()
                .child("Display duration (your desktop may override it)")
                .into_any_element(),
            duration.into_any_element(),
            div()
                .text_sm()
                .child("Sound policy for enabled notification events")
                .into_any_element(),
            sounds.into_any_element(),
            div()
                .h_flex()
                .flex_wrap()
                .gap_2()
                .items_center()
                .child(
                    button("test-notification")
                        .label("Test completion notification")
                        .disabled_feedback(self.notification_testing)
                        .on_click(cx.listener(|this, _, _, cx| this.test_notifications(false, cx))),
                )
                .child(
                    button("test-failure-notification")
                        .label("Test failure notification")
                        .disabled_feedback(self.notification_testing)
                        .on_click(cx.listener(|this, _, _, cx| this.test_notifications(true, cx))),
                )
                .children(self.notification_feedback.as_ref().map(|(tone, _)| {
                    crate::ui::chip(
                        *tone,
                        if self.notification_testing {
                            Some(IconName::Loader)
                        } else {
                            None
                        },
                        match (*tone, self.notification_testing) {
                            (_, true) => "Sending",
                            (crate::ui::Tone::Success, _) => "Sent",
                            _ => "Failed",
                        },
                        cx,
                    )
                }))
                .into_any_element(),
            div()
                .text_sm()
                .child(self.notification_feedback.as_ref().map_or_else(
                    || "The test uses unsaved values; saving is not required.".to_owned(),
                    |(_, message)| message.clone(),
                ))
                .into_any_element(),
        ];
        let notifications = section(
            "Notifications and sounds",
            "One notification per batch while the app is running. Desktop do-not-disturb settings apply.",
            notification_controls,
            cx,
        );
        let mut keys = div().h_flex().flex_wrap().gap_2();
        for (id, label, value) in [
            ("keys-full", "All shortcuts", ShortcutMode::Full),
            ("keys-nav", "Navigation only", ShortcutMode::Navigation),
            ("keys-off", "Shortcuts off", ShortcutMode::Off),
        ] {
            keys = keys.child(self.option(id, label, p.shortcuts == value, cx, move |p| {
                p.shortcuts = value
            }));
        }
        let shortcut_controls=vec![keys.into_any_element(),
            div().text_sm().child("Navigation: Ctrl+1…5, Alt+Up and F5. Full mode adds search, Add, Commands and Help shortcuts.").into_any_element(),
            self.toggle_option("context-menu", "Enable file context menus",p.context_menus,cx,|p,v|p.context_menus=v),
            self.toggle_option("shortcut-hints", "Show shortcut hints in tooltips and help",p.shortcut_hints,cx,|p,v|p.shortcut_hints=v)];
        let shortcuts = section(
            "Keyboard and context menus",
            "Commands and Help are available from the sidebar.",
            shortcut_controls,
            cx,
        );
        let help_controls = vec![
            self.toggle_option(
                "inline-help",
                "Show contextual explanations in Files",
                p.inline_help,
                cx,
                |p, v| p.inline_help = v,
            ),
            self.toggle_option(
                "first-run-help",
                "Show first-account guidance",
                p.first_run_help,
                cx,
                |p, v| p.first_run_help = v,
            ),
            div()
                .text_sm()
                .child("Required warnings and confirmations stay visible.")
                .into_any_element(),
        ];
        let help = section(
            "Guidance and help",
            "Choose where contextual guidance appears.",
            help_controls,
            cx,
        );
        let tray_status = self
            .backend
            .tray
            .as_ref()
            .map(|tray| tray.status())
            .unwrap_or_else(|| "Unavailable".into());
        let tray = section("System tray", "Requires StatusNotifier support. GNOME may need an AppIndicator extension. Closing the window exits the app and stops transfers.", vec![
            self.toggle_option("tray-enabled", "Show a system tray icon", p.tray_enabled, cx, |p,v| p.tray_enabled=v),
            self.toggle_option("tray-idle", "Keep the icon visible when no transfers are active", p.tray_when_idle, cx, |p,v| p.tray_when_idle=v),
            self.toggle_option("tray-badge", "Show a transfer count badge", p.tray_badge, cx, |p,v| p.tray_badge=v),
            self.toggle_option("tray-primary", "Primary click opens Transfers; otherwise show the current screen", p.tray_open_transfers, cx, |p,v| p.tray_open_transfers=v),
            div().text_sm().child(format!("Tray status: {tray_status}. The badge shows up to 99 queued and running transfers. Hover over the icon for the exact count.")).into_any_element(),
            button("retry-tray").label("Retry tray connection").disabled_feedback(self.saving() || !self.config.as_ref().is_some_and(|c| c.desktop().tray_enabled)).on_click(cx.listener(|this,_,_,cx| { if let Some(tray) = &this.backend.tray { tray.retry(); } cx.notify(); })).into_any_element(),
        ],cx);
        div()
            .w_full()
            .max_w(px(560.0))
            .child(appearance)
            .child(browsing)
            .child(notifications)
            .child(tray)
            .child(shortcuts)
            .child(help)
            .children(status_line(
                &self.desktop_status,
                cx.theme().muted_foreground,
                cx.theme().danger,
                self.feedback_revision,
                cx,
            ))
            .pb_4()
            .into_any_element()
    }

    /// Whether the desktop preferences differ from what is saved.
    pub(super) fn desktop_changed(&self) -> bool {
        self.config
            .as_ref()
            .is_some_and(|config| config.desktop() != &self.desktop_draft)
    }

    /// Stays below the scrolling page while there is something to save, so
    /// Save and Discard never scroll out of reach.
    pub(super) fn save_bar(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let changed = self.desktop_changed();
        let saving = self.saving();
        changed.then(|| {
            div()
                .h_flex()
                .flex_none()
                .flex_wrap()
                .items_center()
                .gap_2()
                .px_6()
                .py_3()
                .border_t_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().background)
                .child(
                    div()
                        .size(px(8.0))
                        .flex_none()
                        .rounded_full()
                        .bg(cx.theme().primary),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(160.0))
                        .font_semibold()
                        .child("Unsaved desktop preferences"),
                )
                .child(
                    button("discard-desktop-options")
                        .label("Discard")
                        .disabled_feedback(saving)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(config) = &this.config {
                                this.desktop_draft = config.desktop().clone();
                            }
                            this.desktop_status = Status::Idle;
                            cx.notify();
                        })),
                )
                .child(
                    button("save-desktop-options")
                        .primary()
                        .label("Save desktop preferences")
                        .disabled_feedback(saving)
                        .on_click(cx.listener(|this, _, _, cx| this.save_desktop(cx))),
                )
                .into_any_element()
        })
    }
}
