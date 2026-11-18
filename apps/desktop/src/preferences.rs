//! Live desktop preferences, applied only after a successful configuration save.
use bufonidae_core::config::DesktopOptions;
use gpui::{App, Global, px};
use gpui_component::theme::Theme;

pub struct Preferences(pub DesktopOptions);
impl Global for Preferences {}

pub fn current(cx: &App) -> DesktopOptions {
    cx.try_global::<Preferences>()
        .map(|preferences| preferences.0.clone())
        .unwrap_or_default()
}

pub fn apply(options: DesktopOptions, cx: &mut App) {
    Theme::global_mut(cx).font_size = px(f32::from(options.text_size));
    cx.set_global(Preferences(options));
}

pub fn hint(cx: &App, label: &str, keys: &str, navigation: bool) -> String {
    use bufonidae_core::config::ShortcutMode;
    let options = current(cx);
    if options.shortcut_hints
        && (options.shortcuts == ShortcutMode::Full
            || (navigation && options.shortcuts == ShortcutMode::Navigation))
    {
        format!("{label} ({keys})")
    } else {
        label.to_owned()
    }
}
