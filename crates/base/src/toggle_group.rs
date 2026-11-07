use gpui::{
    AnyElement, App, Axis, Div, ElementId, InteractiveElement, Interactivity, IntoElement,
    KeyDownEvent, ParentElement, RenderOnce, Role, Stateful, StatefulInteractiveElement,
    StyleRefinement, Styled, Window, accesskit, div, prelude::FluentBuilder as _,
};
use smallvec::SmallVec;

use crate::{RovingFocus, StyledExt as _};

/// An unstyled container for a set of toggle elements.
///
/// Given the group's [`RovingFocus`] ([`ToggleGroup::roving_focus`]), whose
/// handles its toggles track, the group is one stop in the window's Tab order.
/// The arrow keys along the roving focus's orientation move focus between the
/// enabled toggles, wrapping at either end, and Home and End jump to the first
/// and last one; moving never presses a toggle. Enter or Space presses the
/// focused toggle, as in Base UI's Toggle Group.
#[derive(IntoElement)]
pub struct ToggleGroup {
    base: Stateful<Div>,
    style: StyleRefinement,
    axis: Axis,
    children: SmallVec<[AnyElement; 4]>,
    roving_focus: Option<RovingFocus>,
}

impl ToggleGroup {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(id.into()),
            style: StyleRefinement::default(),
            axis: Axis::Horizontal,
            children: SmallVec::new(),
            roving_focus: None,
        }
    }

    /// Sets the semantic axis of the group.
    pub fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }

    /// The roving focus over the group's toggles, in order. Each toggle
    /// tracks the matching [`RovingFocus::focus_handle`] and tab stop.
    pub fn roving_focus(mut self, roving_focus: RovingFocus) -> Self {
        self.roving_focus = Some(roving_focus);
        self
    }
}

impl Styled for ToggleGroup {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for ToggleGroup {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl InteractiveElement for ToggleGroup {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for ToggleGroup {}

impl RenderOnce for ToggleGroup {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.base
            .role(Role::Toolbar)
            .aria_orientation(match self.axis {
                Axis::Horizontal => accesskit::Orientation::Horizontal,
                Axis::Vertical => accesskit::Orientation::Vertical,
            })
            .when_some(self.roving_focus, |this, roving_focus| {
                this.on_key_down(move |event: &KeyDownEvent, window, cx| {
                    roving_focus.handle_key_down(event, window, cx);
                })
            })
            .children(self.children)
            .refine_style(&self.style)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Toggle;
    use gpui::{Context, FocusHandle, Render, TestAppContext, VisualTestContext, px};

    /// Three toggles in a row, the last one pressed.
    struct Formatting {
        pressed: [bool; 3],
        handles: Vec<FocusHandle>,
    }

    impl Render for Formatting {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let roving = RovingFocus::new("formatting-roving", 3, window, cx);
            self.handles = (0..3).map(|ix| roving.focus_handle(ix)).collect();
            ToggleGroup::new("formatting")
                .flex()
                .roving_focus(roving.clone())
                .children((0..3).map(|ix| {
                    Toggle::new(ix)
                        .size(px(20.))
                        .pressed(self.pressed[ix])
                        .track_focus(&self.handles[ix])
                        .tab_stop(roving.is_tab_stop(ix))
                }))
        }
    }

    fn formatting(cx: &mut TestAppContext) -> (gpui::Entity<Formatting>, &mut VisualTestContext) {
        let (view, cx) = cx.add_window_view(|_, _| Formatting {
            pressed: [false, false, true],
            handles: Vec::new(),
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            window.focus_next(cx);
            window.draw(cx).clear(cx);
        });
        (view, cx)
    }

    fn focused(view: &gpui::Entity<Formatting>, cx: &mut VisualTestContext) -> Option<usize> {
        cx.update(|window, cx| {
            view.read(cx)
                .handles
                .iter()
                .position(|handle| handle.is_focused(window))
        })
    }

    #[gpui::test]
    fn a_toggle_group_is_one_tab_stop(cx: &mut TestAppContext) {
        let (view, cx) = formatting(cx);
        assert_eq!(focused(&view, cx), Some(0), "Tab lands on the first toggle");

        let stops: Vec<bool> = view.read_with(cx, |group, _| {
            group.handles.iter().map(|handle| handle.tab_stop).collect()
        });
        assert_eq!(stops, vec![true, false, false]);
    }

    #[gpui::test]
    fn arrow_keys_move_focus_between_toggles_without_pressing_them(cx: &mut TestAppContext) {
        let (view, cx) = formatting(cx);

        cx.simulate_keystrokes("right right");
        assert_eq!(focused(&view, cx), Some(2));
        cx.simulate_keystrokes("right");
        assert_eq!(
            focused(&view, cx),
            Some(0),
            "past the last toggle focus wraps"
        );
        cx.simulate_keystrokes("end");
        assert_eq!(focused(&view, cx), Some(2));
        assert_eq!(
            view.read_with(cx, |group, _| group.pressed),
            [false, false, true]
        );
    }
}
