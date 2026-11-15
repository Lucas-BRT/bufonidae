//! The Bufonidae look: soft modern, warm neutrals and a red accent, in a light
//! and a dark variant. The app follows the system appearance and the system
//! UI font (`.SystemUIFont`, the component library's default).

use std::rc::Rc;

use gpui::{App, Window};
use gpui_component::theme::{Theme, ThemeSet};

const THEME: &str = include_str!("bufonidae-theme.json");

/// Registers the light and dark variants. Call once, after
/// `gpui_component::init`.
pub fn install(cx: &mut App) {
    let set: ThemeSet = serde_json::from_str(THEME).expect("the bundled theme is valid JSON");
    let theme = Theme::global_mut(cx);
    for config in set.themes {
        theme.apply_config(&Rc::new(config));
    }
}

/// Applies the variant that matches the system, now and whenever it changes.
pub fn follow_system(window: &mut Window, cx: &mut App) {
    Theme::sync_system_appearance(Some(window), cx);
}
