//! Shared presentation pieces: status chips, notices and empty states. Every
//! state carries an icon and text as well as a color, so none depends on hue.

use gpui::{AnyElement, App, IntoElement, ParentElement, Styled as _, div, px};
use gpui_component::{ActiveTheme as _, Icon, IconName, StyledExt as _};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Neutral,
    Success,
    Warning,
    Danger,
}

impl Tone {
    pub fn icon(self) -> IconName {
        match self {
            Self::Neutral => IconName::Info,
            Self::Success => IconName::CircleCheck,
            Self::Warning => IconName::TriangleAlert,
            Self::Danger => IconName::CircleX,
        }
    }

    /// The accent for borders and tints. Text always stays in the foreground
    /// color, so contrast never depends on the accent.
    pub fn accent(self, cx: &App) -> gpui::Hsla {
        let theme = cx.theme();
        match self {
            Self::Neutral => theme.muted_foreground,
            Self::Success => theme.success,
            Self::Warning => theme.warning,
            Self::Danger => theme.danger,
        }
    }
}

/// A small pill: tone icon plus a short label.
pub fn chip(tone: Tone, icon: Option<IconName>, label: impl Into<String>, cx: &App) -> AnyElement {
    let accent = tone.accent(cx);
    div()
        .h_flex()
        .flex_none()
        .items_center()
        .gap_1()
        .px_2()
        .py_0p5()
        .rounded_full()
        .border_1()
        .border_color(accent)
        .bg(accent.opacity(0.12))
        .text_sm()
        .child(Icon::new(icon.unwrap_or_else(|| tone.icon())).size_3p5())
        .child(label.into())
        .into_any_element()
}

/// An inline message with icon, text and an optional action beside it.
pub fn notice(
    tone: Tone,
    text: impl Into<String>,
    action: Option<AnyElement>,
    cx: &App,
) -> gpui::Div {
    let accent = tone.accent(cx);
    div()
        .h_flex()
        .items_center()
        .gap_3()
        .p_3()
        .rounded(px(10.0))
        .border_1()
        .border_color(accent)
        .bg(accent.opacity(0.08))
        .child(
            Icon::new(tone.icon())
                .size_4()
                .text_color(accent)
                .flex_none(),
        )
        .child(div().flex_1().min_w(px(0.0)).child(text.into()))
        .children(action)
}

/// A small badge with a count, for navigation items.
pub fn count_badge(count: usize, cx: &App) -> AnyElement {
    div()
        .flex_none()
        .min_w(px(20.0))
        .h(px(20.0))
        .px_1p5()
        .rounded_full()
        .bg(cx.theme().primary)
        .text_color(cx.theme().primary_foreground)
        .text_xs()
        .font_semibold()
        .flex()
        .items_center()
        .justify_center()
        .child(if count > 99 {
            "99+".to_owned()
        } else {
            count.to_string()
        })
        .into_any_element()
}

/// A modal card whose title and content scroll while the footer buttons stay
/// put, so Cancel and the primary action are always reachable.
#[derive(IntoElement)]
pub struct Dialog {
    width: f32,
    max_width: gpui::Pixels,
    max_height: gpui::Pixels,
    surface: gpui::Hsla,
    border: gpui::Hsla,
    body: Vec<AnyElement>,
    footer: Option<AnyElement>,
}

impl Dialog {
    pub fn new(width: f32, window: &gpui::Window, cx: &App) -> Self {
        let viewport = window.viewport_size();
        Self {
            width,
            max_width: viewport.width - px(32.0),
            max_height: viewport.height - px(32.0),
            surface: cx.theme().popover,
            border: cx.theme().border,
            body: Vec::new(),
            footer: None,
        }
    }

    pub fn footer(mut self, footer: impl IntoElement) -> Self {
        self.footer = Some(footer.into_any_element());
        self
    }
}

impl ParentElement for Dialog {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.body.extend(elements);
    }
}

impl gpui::RenderOnce for Dialog {
    fn render(self, _: &mut gpui::Window, _: &mut App) -> impl IntoElement {
        use gpui::InteractiveElement as _;
        use gpui::StatefulInteractiveElement as _;
        div()
            .v_flex()
            .w(px(self.width))
            .max_w(self.max_width)
            .max_h(self.max_height)
            .gap_4()
            .p_6()
            .rounded(px(14.0))
            .bg(self.surface)
            .border_1()
            .border_color(self.border)
            .shadow_lg()
            .child(
                div()
                    .id("dialog-body")
                    .v_flex()
                    .w_full()
                    .min_w(px(0.0))
                    .gap_4()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .children(self.body),
            )
            .children(self.footer.map(|footer| div().flex_none().child(footer)))
    }
}
