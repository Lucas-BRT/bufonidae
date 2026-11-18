//! Shared pointer affordances; toolkit hover, pressed, focus and click behavior
//! stay in the native controls. Disabled controls never advertise a click.

use gpui::{ElementId, Styled};
use gpui_component::{Disableable, button::Button, checkbox::Checkbox};

pub fn button(id: impl Into<ElementId>) -> Button {
    Button::new(id).cursor_pointer()
}

pub fn checkbox(id: impl Into<ElementId>) -> Checkbox {
    Checkbox::new(id).cursor_pointer()
}

pub trait ControlFeedback: Disableable + Styled + Sized {
    fn disabled_feedback(self, disabled: bool) -> Self {
        let control = self.disabled(disabled);
        if disabled {
            control.cursor_default()
        } else {
            control
        }
    }
}

impl<T: Disableable + Styled> ControlFeedback for T {}

/// Native keyboard-operable buttons inside the existing collapsible sidebar.
#[derive(gpui::IntoElement)]
pub struct Navigation {
    buttons: Vec<Button>,
    collapsed: bool,
}

impl Navigation {
    pub fn new(buttons: Vec<Button>) -> Self {
        Self {
            buttons,
            collapsed: false,
        }
    }
}

impl gpui_component::Collapsible for Navigation {
    fn is_collapsed(&self) -> bool {
        self.collapsed
    }
    fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }
}

impl gpui::RenderOnce for Navigation {
    fn render(self, _: &mut gpui::Window, _: &mut gpui::App) -> impl gpui::IntoElement {
        use gpui::ParentElement as _;
        use gpui_component::StyledExt as _;
        gpui::div()
            .v_flex()
            .gap_2()
            .children(self.buttons.into_iter().map(|button| {
                if self.collapsed {
                    button.label("").w_8()
                } else {
                    button
                }
            }))
    }
}
