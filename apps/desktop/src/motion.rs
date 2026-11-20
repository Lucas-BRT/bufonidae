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

/// Decelerating curve for things that arrive.
pub fn ease_out(value: f32) -> f32 {
    1.0 - (1.0 - value).powi(3)
}

/// Accelerating curve for things that leave.
pub fn ease_in(value: f32) -> f32 {
    value.powi(3)
}

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
            Animation::new(if leaving { EXIT } else { ENTER }).with_easing(move |value| {
                if leaving {
                    ease_in(value)
                } else {
                    ease_out(value)
                }
            }),
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

/// The dimming layer behind a dialog: fades with the dialog and leaves with
/// the same key, so scrim and card never get out of step.
pub fn scrim(
    color: gpui::Hsla,
    id: impl Into<ElementId>,
    leaving: bool,
    exit_start: f32,
    cx: &App,
) -> AnyElement {
    let layer = div().absolute().inset_0().bg(color);
    if !enabled(cx) {
        return layer.into_any_element();
    }
    layer
        .with_animation(id, transition(leaving), move |element, value| {
            element.opacity(if leaving {
                exit_start * (1.0 - value)
            } else {
                value
            })
        })
        .into_any_element()
}

/// A dialog card: fades and rises 8 px on entry, and sinks back on exit.
pub fn dialog(
    content: impl IntoElement,
    id: impl Into<ElementId>,
    leaving: bool,
    exit_start: f32,
    cx: &App,
) -> AnyElement {
    let card = div().relative().child(content);
    if !enabled(cx) {
        return card.into_any_element();
    }
    card.with_animation(id, transition(leaving), move |element, value| {
        let (opacity, offset) = if leaving {
            (exit_start * (1.0 - value), 8.0 * value)
        } else {
            (value, 8.0 * (1.0 - value))
        };
        element.opacity(opacity).top(px(offset))
    })
    .into_any_element()
}

fn transition(leaving: bool) -> Animation {
    Animation::new(if leaving { EXIT } else { ENTER }).with_easing(move |value| {
        if leaving {
            ease_in(value)
        } else {
            ease_out(value)
        }
    })
}

/// Shows `content` only after a short wait, so a load that finishes at once
/// never flashes an indicator. Reduced motion shows it immediately.
pub fn delayed(content: AnyElement, id: impl Into<ElementId>, cx: &App) -> AnyElement {
    const WAIT_MS: f32 = 250.0;
    const FADE_MS: f32 = 200.0;
    let content = div().size_full().child(content);
    if !enabled(cx) {
        return content.into_any_element();
    }
    content
        .with_animation(
            id,
            Animation::new(Duration::from_millis(450)),
            |element, value| element.opacity(((value * 450.0 - WAIT_MS) / FADE_MS).clamp(0.0, 1.0)),
        )
        .into_any_element()
}

/// A usage bar that fills from empty when it first appears. The id keeps the
/// entrance from replaying on refresh; a later value change just moves it.
pub fn fill(
    id: impl Into<ElementId>,
    percent: f32,
    color: Option<gpui::Hsla>,
    cx: &App,
) -> AnyElement {
    let bar = match color {
        Some(color) => Progress::new().bg(color),
        None => Progress::new(),
    };
    if !enabled(cx) {
        return bar.value(percent).into_any_element();
    }
    bar.with_animation(
        id,
        Animation::new(Duration::from_millis(400)).with_easing(|value| 1.0 - (1.0 - value).powi(3)),
        move |bar, value| bar.value(percent * value),
    )
    .into_any_element()
}

/// A tint over a row that fades out in about a second: "this just arrived".
/// Nothing is drawn when motion is reduced; the row is still in the list.
pub fn flash(id: impl Into<ElementId>, cx: &App) -> Option<AnyElement> {
    enabled(cx).then(|| {
        div()
            .absolute()
            .inset_0()
            .bg(cx.theme().accent)
            .with_animation(
                id,
                Animation::new(Duration::from_millis(1200)),
                |element, value| element.opacity(0.6 * (1.0 - value).powi(2)),
            )
            .into_any_element()
    })
}

/// How long the "New" chip marks arrivals when motion is reduced.
pub const NEW_CHIP: Duration = Duration::from_secs(5);

/// How long a `flash` stays mounted.
pub const FLASH: Duration = Duration::from_millis(1300);

/// Fades `content` in after a short delay that grows with `index` (30 ms a
/// step, capped at six steps), for lists that appear all at once.
pub fn stagger(
    content: AnyElement,
    id: impl Into<ElementId>,
    index: usize,
    cx: &App,
) -> AnyElement {
    let content = div().child(content);
    if !enabled(cx) {
        return content.into_any_element();
    }
    let delay = 30.0 * index.min(6) as f32;
    let total = delay + 160.0;
    content
        .with_animation(
            id,
            Animation::new(Duration::from_millis(total as u64)),
            move |element, value| {
                element.opacity(ease_out(((value * total - delay) / 160.0).clamp(0.0, 1.0)))
            },
        )
        .into_any_element()
}

/// Reveals `content` by growing its height up to `max` over the enter
/// duration; with reduced motion it is simply there. The id restarts it.
pub fn reveal(content: AnyElement, id: impl Into<ElementId>, max: f32, cx: &App) -> AnyElement {
    let content = div().overflow_hidden().child(content);
    if !enabled(cx) {
        return content.max_h(px(max)).into_any_element();
    }
    content
        .with_animation(
            id,
            Animation::new(ENTER).with_easing(ease_out),
            move |element, value| element.max_h(px(max * value)).opacity(value),
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

    #[test]
    fn exit_accelerates_and_entrance_decelerates() {
        assert_eq!((ease_in(0.0), ease_in(1.0)), (0.0, 1.0));
        assert_eq!((ease_out(0.0), ease_out(1.0)), (0.0, 1.0));
        let mut last = (0.0, 0.0);
        for step in 1..=20 {
            let v = step as f32 / 20.0;
            assert!(ease_in(v) > last.0 && ease_out(v) > last.1);
            last = (ease_in(v), ease_out(v));
        }
        assert!(ease_in(0.5) < 0.5 && ease_out(0.5) > 0.5);
    }
}
