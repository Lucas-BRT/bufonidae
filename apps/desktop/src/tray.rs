//! Native StatusNotifierItem integration, isolated from the UI executor.
use bufonidae_core::{config::DesktopOptions, index::Index};
use ksni::TrayMethods;
use std::sync::{Arc, Mutex, mpsc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Show,
    Transfers,
    Settings,
}

pub struct Service {
    options: tokio::sync::watch::Sender<DesktopOptions>,
    actions: Mutex<mpsc::Receiver<Action>>,
    status: Arc<Mutex<String>>,
}

impl Service {
    pub fn new(runtime: &tokio::runtime::Runtime, index: Arc<Index>) -> Self {
        let (options, mut changes) = tokio::sync::watch::channel(DesktopOptions::default());
        let (sender, receiver) = mpsc::channel();
        let status = Arc::new(Mutex::new(String::from("Disabled")));
        let report = Arc::clone(&status);
        runtime.spawn(async move {
            let mut handle: Option<ksni::Handle<TrayIcon>> = None;
            let mut attempted = false;
            loop {
                let p = changes.borrow_and_update().clone();
                let active = if !p.tray_enabled { 0 } else { match index.jobs().await {
                    Ok(jobs) => jobs.iter().filter(|j| matches!(j.state.as_str(), "queued" | "running")).count(),
                    Err(_) => {
                        set_status(&report, "Could not read transfer status. Tray status will retry.");
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        continue;
                    }
                }};
                let visible = p.tray_enabled && (p.tray_when_idle || active > 0);
                if !visible {
                    if let Some(icon) = handle.take() { icon.shutdown().await; }
                    attempted = false;
                    set_status(&report, if p.tray_enabled { "Hidden while idle" } else { "Disabled" });
                } else if handle.as_ref().is_some_and(|h| h.is_closed()) {
                    handle = None;
                    set_status(&report, "Tray connection closed. Use Retry tray connection.");
                } else if let Some(icon) = &handle {
                    icon.update(|icon| { icon.active = active; icon.badge = p.tray_badge; icon.open_transfers = p.tray_open_transfers; }).await;
                } else if !attempted {
                    attempted = true;
                    set_status(&report, "Connecting to the desktop tray service…");
                    let icon = TrayIcon { active, badge: p.tray_badge, open_transfers: p.tray_open_transfers, actions: sender.clone() };
                    match tokio::time::timeout(std::time::Duration::from_secs(3), icon.spawn()).await {
                        Ok(Ok(icon)) => { handle = Some(icon); set_status(&report, "Connected to the desktop tray service"); }
                        Ok(Err(error)) => set_status(&report, &format!("Tray unavailable: {error}. Check desktop StatusNotifier support, then Retry.")),
                        Err(_) => set_status(&report, "Tray connection timed out. Check desktop StatusNotifier support, then Retry."),
                    }
                }
                tokio::select! {
                    changed = changes.changed() => { if changed.is_err() { break; } attempted = false; }
                    _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
                }
            }
            if let Some(icon) = handle { icon.shutdown().await; }
        });
        Self {
            options,
            actions: Mutex::new(receiver),
            status,
        }
    }

    pub fn configure(&self, options: DesktopOptions) {
        self.options.send_if_modified(|current| {
            if *current == options {
                false
            } else {
                *current = options;
                true
            }
        });
    }
    pub fn retry(&self) {
        self.options.send_modify(|_| {});
    }
    pub fn action(&self) -> Option<Action> {
        self.actions.lock().ok()?.try_recv().ok()
    }
    pub fn status(&self) -> String {
        self.status
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|_| "Tray status unavailable".into())
    }
}

fn set_status(status: &Mutex<String>, message: &str) {
    if let Ok(mut status) = status.lock() {
        *status = message.into();
    }
}

struct TrayIcon {
    active: usize,
    badge: bool,
    open_transfers: bool,
    actions: mpsc::Sender<Action>,
}
impl ksni::Tray for TrayIcon {
    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        false
    }
    fn id(&self) -> String {
        "bufonidae".into()
    }
    fn title(&self) -> String {
        format!("Bufonidae · {} active transfers", self.active)
    }
    fn icon_name(&self) -> String {
        "folder-remote".into()
    }
    fn activate(&mut self, _: i32, _: i32) {
        let _ = self.actions.send(if self.open_transfers {
            Action::Transfers
        } else {
            Action::Show
        });
    }
    fn overlay_icon_pixmap(&self) -> Vec<ksni::Icon> {
        if self.badge && self.active > 0 {
            vec![badge(self.active)]
        } else {
            vec![]
        }
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Bufonidae".into(),
            description: format!("{} queued or running transfers", self.active),
            ..Default::default()
        }
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        [
            ("Show Bufonidae", Action::Show),
            ("Transfers", Action::Transfers),
            ("Settings", Action::Settings),
        ]
        .into_iter()
        .map(|(label, action)| {
            ksni::menu::StandardItem {
                label: label.into(),
                activate: Box::new(move |tray: &mut Self| {
                    let _ = tray.actions.send(action);
                }),
                ..Default::default()
            }
            .into()
        })
        .collect()
    }
}

fn badge(count: usize) -> ksni::Icon {
    const DIGITS: [[u8; 5]; 10] = [
        [7, 5, 5, 5, 7],
        [2, 6, 2, 2, 7],
        [7, 1, 7, 4, 7],
        [7, 1, 7, 1, 7],
        [5, 5, 7, 1, 1],
        [7, 4, 7, 1, 7],
        [7, 4, 7, 5, 7],
        [7, 1, 1, 1, 1],
        [7, 5, 7, 5, 7],
        [7, 5, 7, 1, 7],
    ];
    let mut data = vec![0; 16 * 16 * 4];
    // Bottom-right count, capped at 99; tooltip always contains the exact count.
    for y in 8..16 {
        for x in 7..16 {
            data[(y * 16 + x) * 4..(y * 16 + x) * 4 + 4].copy_from_slice(&[255, 32, 32, 32]);
        }
    }
    let number = count.min(99).to_string();
    for (i, digit) in number.bytes().enumerate() {
        for (y, bits) in DIGITS[(digit - b'0') as usize].iter().enumerate() {
            for x in 0..3 {
                if bits & (1 << (2 - x)) != 0 {
                    let offset = ((10 + y) * 16 + 8 + i * 4 + x) * 4;
                    data[offset..offset + 4].copy_from_slice(&[255, 255, 255, 255]);
                }
            }
        }
    }
    ksni::Icon {
        width: 16,
        height: 16,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ksni::Tray;
    #[test]
    #[ignore = "Run under dbus-run-session without a tray watcher"]
    fn missing_watcher_is_reported_and_disabling_recovers() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let index = runtime.block_on(Index::open_memory()).unwrap();
        let service = Service::new(&runtime, Arc::new(index));
        let options = DesktopOptions {
            tray_enabled: true,
            ..DesktopOptions::default()
        };
        service.configure(options);
        runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(6), async {
                loop {
                    let status = service.status();
                    if status.starts_with("Tray unavailable:")
                        || status.starts_with("Tray connection timed out")
                    {
                        break;
                    }
                    assert!(
                        !status.starts_with("Connected"),
                        "Test requires an isolated bus without a watcher"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap();
            service.retry();
            service.configure(DesktopOptions::default());
            tokio::time::timeout(std::time::Duration::from_secs(6), async {
                while service.status() != "Disabled" {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap();
        });
    }

    #[test]
    fn badge_caps_pixels_but_keeps_exact_tooltip_count() {
        assert_eq!(badge(99).data, badge(1000).data);
        assert_eq!(badge(1).data.len(), 16 * 16 * 4);
        let (actions, receiver) = mpsc::channel();
        let mut icon = TrayIcon {
            active: 1000,
            badge: true,
            open_transfers: true,
            actions,
        };
        assert!(icon.tool_tip().description.contains("1000"));
        assert_eq!(icon.overlay_icon_pixmap().len(), 1);
        icon.activate(0, 0);
        assert_eq!(receiver.try_recv().unwrap(), Action::Transfers);
        icon.open_transfers = false;
        icon.activate(0, 0);
        assert_eq!(receiver.try_recv().unwrap(), Action::Show);
        icon.active = 0;
        assert!(icon.overlay_icon_pixmap().is_empty());
        icon.active = 3;
        icon.badge = false;
        assert!(icon.overlay_icon_pixmap().is_empty());
    }
}
