//! Shared motion tokens. Stable animation IDs prevent state updates from
//! restarting an entrance. Reduced motion retains every textual status.

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, ElementId, Global, IntoElement,
    ParentElement as _, Styled as _, div, px,
};
use gpui_component::{ActiveTheme as _, StyledExt as _, progress::Progress};
use std::time::Duration;

pub const ENTER: Duration = Duration::from_millis(160);
pub const EXIT: Duration = Duration::from_millis(120);
const PROGRESS: Duration = Duration::from_millis(180);

pub struct Preferences {
    pub reduced: bool,
}
impl Global for Preferences {}

pub fn enabled(cx: &App) -> bool {
    cx.try_global::<Preferences>()
        .is_some_and(|preferences| !preferences.reduced)
}

pub fn fade(
    element: impl IntoElement,
    id: impl Into<ElementId>,
    leaving: bool,
    exit_start: f32,
    cx: &App,
) -> AnyElement {
    let element = div().child(element);
    if !enabled(cx) {
        return element.into_any_element();
    }
    element
        .with_animation(
            id,
            Animation::new(if leaving { EXIT } else { ENTER })
                .with_easing(|value| 1.0 - (1.0 - value).powi(3)),
            move |element, value| {
                element.opacity(if leaving {
                    exit_start * (1.0 - value)
                } else {
                    value
                })
            },
        )
        .into_any_element()
}

/// Animate navigation only; background refreshes retain the same identity.
pub fn page(content: AnyElement, generation: usize, cx: &App) -> AnyElement {
    let page = div().relative().size_full().child(content);
    if !enabled(cx) {
        return page.into_any_element();
    }
    page.with_animation(
        ("page-enter", generation),
        Animation::new(Duration::from_millis(200)).with_easing(|value| 1.0 - (1.0 - value).powi(3)),
        |page, value| page.opacity(value).top(px(8.0 * (1.0 - value))),
    )
    .into_any_element()
}

pub fn activity(label: &'static str, cx: &App) -> AnyElement {
    let dot = div().size(px(8.0)).rounded_full().bg(cx.theme().primary);
    let dot = if enabled(cx) {
        dot.with_animation(
            label,
            Animation::new(Duration::from_millis(1200)).repeat(),
            |element, value| element.opacity(0.45 + 0.55 * (value * std::f32::consts::PI).sin()),
        )
        .into_any_element()
    } else {
        dot.into_any_element()
    };
    div()
        .h_flex()
        .gap_2()
        .items_center()
        .child(dot)
        .child(div().text_sm().child(label))
        .into_any_element()
}

pub fn progress(from: f32, to: f32, revision: usize, cx: &App) -> AnyElement {
    if !enabled(cx) || (from - to).abs() < f32::EPSILON {
        return Progress::new().value(to).into_any_element();
    }
    Progress::new()
        .with_animation(
            ("completed-progress", revision),
            Animation::new(PROGRESS),
            move |bar, value| bar.value(from + (to - from) * value),
        )
        .into_any_element()
}

/// Completed-byte progress, with a processed-item fallback for empty files.
#[allow(clippy::cast_precision_loss)]
pub fn completion_percent(bytes: u64, total_bytes: u64, items: usize, total_items: usize) -> f32 {
    if total_bytes > 0 {
        (bytes as f64 / total_bytes as f64 * 100.0).clamp(0.0, 100.0) as f32
    } else if total_items > 0 {
        (items as f64 / total_items as f64 * 100.0).clamp(0.0, 100.0) as f32
    } else {
        0.0
    }
}

/// Continue from the visible value when a new completion event interrupts a tween.
pub fn progress_sample(from: f32, to: f32, elapsed: Duration) -> f32 {
    let fraction = (elapsed.as_secs_f32() / PROGRESS.as_secs_f32()).clamp(0.0, 1.0);
    from + (to - from) * fraction
}

pub fn entrance_opacity(elapsed: Duration) -> f32 {
    let fraction = (elapsed.as_secs_f32() / ENTER.as_secs_f32()).clamp(0.0, 1.0);
    1.0 - (1.0 - fraction).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_and_empty_transfers_never_invent_progress() {
        assert_eq!(completion_percent(0, 0, 0, 0), 0.0);
        assert_eq!(completion_percent(0, 0, 1, 4), 25.0);
        assert_eq!(completion_percent(20, 100, 4, 4), 20.0);
        assert_eq!(completion_percent(u64::MAX, 1, 0, 0), 100.0);
    }

    #[test]
    fn interrupted_progress_continues_from_the_visible_value() {
        let visible = progress_sample(0.0, 50.0, Duration::from_millis(90));
        assert!((visible - 25.0).abs() < 0.001);
        assert_eq!(progress_sample(visible, 75.0, Duration::ZERO), visible);
        assert_eq!(progress_sample(visible, 75.0, Duration::from_secs(1)), 75.0);
        assert_eq!(entrance_opacity(Duration::ZERO), 0.0);
        assert_eq!(entrance_opacity(ENTER), 1.0);
    }
}
