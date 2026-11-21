mod accounts;
mod add_account;
mod app;
mod backend;
mod browse;
mod check;
mod commands;
mod distribution;
mod download;
mod files;
mod format;
mod interaction;
mod motion;
mod notifications;
mod preferences;
mod remove_account;
mod settings;
mod storage;
mod store;
mod theme;
mod transfers;
mod tray;
mod tree;
mod ui;

use std::borrow::Cow;
use std::sync::Arc;

use gpui::{
    Application, AssetSource, Bounds, SharedString, TitlebarOptions, WindowBounds, WindowOptions,
    prelude::*, px, size,
};
use gpui_component::Root;

const ROTATE_CW_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/></svg>"#;

struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if path == "icons/rotate-cw.svg" {
            return Ok(Some(Cow::Borrowed(ROTATE_CW_SVG)));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        let mut items = gpui_component_assets::Assets.list(path)?;
        if "icons/rotate-cw.svg".starts_with(path) {
            items.push("icons/rotate-cw.svg".into());
        }
        Ok(items)
    }
}

use crate::app::BufonidaeApp;
use crate::backend::Backend;

fn main() {
    // The index is opened before any window exists, so that a second copy of
    // the app can say so instead of failing halfway.
    let backend = Backend::open().map(Arc::new);
    let saved_config = bufonidae_core::config::Config::default_path()
        .and_then(bufonidae_core::config::Config::load)
        .ok();
    let desktop_options = saved_config
        .as_ref()
        .map(|config| config.desktop().clone())
        .unwrap_or_default();
    let reduced_motion = saved_config
        .as_ref()
        .is_some_and(|config| config.reduced_motion());

    Application::new().with_assets(AppAssets).run(move |cx| {
        gpui_component::init(cx);
        theme::install(cx);
        preferences::apply(desktop_options.clone(), cx);
        cx.set_global(motion::Preferences {
            reduced: reduced_motion,
        });

        let bounds = Bounds::centered(None, size(px(1040.0), px(680.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                // Matches `bufonidae.desktop`, so the shell shows the right
                // icon and groups the windows.
                app_id: Some("bufonidae".to_owned()),
                window_min_size: Some(size(px(720.0), px(480.0))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Bufonidae".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                theme::follow_system(window, cx);
                let view = cx.new(|cx| BufonidaeApp::new(backend, window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .expect("could not open the window");
        cx.activate(true);
    });
}
