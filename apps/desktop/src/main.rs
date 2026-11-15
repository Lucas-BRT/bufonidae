mod accounts;
mod add_account;
mod app;
mod backend;
mod browse;
mod check;
mod download;
mod files;
mod format;
mod remove_account;
mod settings;
mod store;
mod theme;
mod tree;

use std::sync::Arc;

use gpui::{
    Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, prelude::*, px, size,
};
use gpui_component::Root;

use crate::app::BufonidaeApp;
use crate::backend::Backend;

fn main() {
    // The index is opened before any window exists, so that a second copy of
    // the app can say so instead of failing halfway.
    let backend = Backend::open().map(Arc::new);

    Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx| {
            gpui_component::init(cx);
            theme::install(cx);

            let bounds = Bounds::centered(None, size(px(1040.0), px(680.0)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
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
