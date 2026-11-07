use std::rc::Rc;

use gpui::{
    AnyElement, App, Axis, Div, ElementId, InteractiveElement, Interactivity, IntoElement,
    KeyDownEvent, ParentElement, RenderOnce, Role, Stateful, StatefulInteractiveElement,
    StyleRefinement, Styled, Window, accesskit, div, prelude::FluentBuilder as _,
};
use smallvec::SmallVec;

use crate::{RovingFocus, StyledExt as _};

type ChangeHandler = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// An unstyled container for a set of radio elements.
///
/// Given the group's [`RovingFocus`] ([`RadioGroup::roving_focus`]), whose
/// handles its radios track, the group is one stop in the window's Tab order,
/// landing on the checked radio (or the first enabled one when none is
/// checked). The arrow keys move focus to the previous or next enabled radio,
/// wrapping at either end, and select it through [`RadioGroup::on_change`]:
/// in a radio group, moving is choosing, as in Base UI's Radio Group. Home and
/// End jump to the first and last enabled radio. Build the roving focus with
/// [`crate::RovingOrientation::Both`] so all four arrows work whatever the
/// layout.
#[derive(IntoElement)]
pub struct RadioGroup {
    base: Stateful<Div>,
    style: StyleRefinement,
    axis: Axis,
    children: SmallVec<[AnyElement; 4]>,
    roving_focus: Option<RovingFocus>,
    on_change: Option<ChangeHandler>,
}

impl RadioGroup {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(id.into()),
            style: StyleRefinement::default(),
            axis: Axis::Vertical,
            children: SmallVec::new(),
            roving_focus: None,
            on_change: None,
        }
    }

    /// Sets the semantic axis of the group.
    pub fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }

    /// The roving focus over the group's radios, in order. Each radio tracks
    /// the matching [`RovingFocus::focus_handle`] and tab stop.
    pub fn roving_focus(mut self, roving_focus: RovingFocus) -> Self {
        self.roving_focus = Some(roving_focus);
        self
    }

    /// Handles a request to select the radio at an index, made when the
    /// keyboard moves focus onto it. This is a controlled value: the owner
    /// writes it and renders the radio checked.
    pub fn on_change(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl Styled for RadioGroup {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for RadioGroup {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl InteractiveElement for RadioGroup {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for RadioGroup {}

impl RenderOnce for RadioGroup {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let on_change = self.on_change;
        self.base
            .role(Role::RadioGroup)
            .aria_orientation(match self.axis {
                Axis::Horizontal => accesskit::Orientation::Horizontal,
                Axis::Vertical => accesskit::Orientation::Vertical,
            })
            .when_some(self.roving_focus, |this, roving_focus| {
                this.on_key_down(move |event: &KeyDownEvent, window, cx| {
                    if let Some(ix) = roving_focus.handle_key_down(event, window, cx)
                        && let Some(on_change) = &on_change
                    {
                        on_change(ix, window, cx);
                    }
                })
            })
            .children(self.children)
            .refine_style(&self.style)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Radio, RovingOrientation};
    use gpui::{Context, FocusHandle, Render, TestAppContext, VisualTestContext, px};

    /// Three radios, the second disabled.
    struct Choices {
        checked: Option<usize>,
        handles: Vec<FocusHandle>,
    }

    impl Render for Choices {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let roving = RovingFocus::new("choices-roving", 3, window, cx)
                .orientation(RovingOrientation::Both)
                .active(self.checked)
                .disabled_items([false, true, false]);
            self.handles = (0..3).map(|ix| roving.focus_handle(ix)).collect();
            let view = cx.entity();
            RadioGroup::new("choices")
                .roving_focus(roving.clone())
                .on_change(move |ix, _, cx| {
                    view.update(cx, |choices, cx| {
                        choices.checked = Some(ix);
                        cx.notify();
                    })
                })
                .children((0..3).map(|ix| {
                    Radio::new(ix)
                        .size(px(20.))
                        .checked(self.checked == Some(ix))
                        .disabled(ix == 1)
                        .track_focus(&self.handles[ix])
                        .tab_stop(roving.is_tab_stop(ix))
                }))
        }
    }

    fn choices(
        checked: Option<usize>,
        cx: &mut TestAppContext,
    ) -> (gpui::Entity<Choices>, &mut VisualTestContext) {
        let (view, cx) = cx.add_window_view(move |_, _| Choices {
            checked,
            handles: Vec::new(),
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            window.focus_next(cx);
            window.draw(cx).clear(cx);
        });
        (view, cx)
    }

    fn focused(view: &gpui::Entity<Choices>, cx: &mut VisualTestContext) -> Option<usize> {
        cx.update(|window, cx| {
            view.read(cx)
                .handles
                .iter()
                .position(|handle| handle.is_focused(window))
        })
    }

    #[gpui::test]
    fn tab_enters_a_radio_group_at_the_checked_radio(cx: &mut TestAppContext) {
        let (view, cx) = choices(Some(2), cx);
        assert_eq!(focused(&view, cx), Some(2));
    }

    #[gpui::test]
    fn tab_enters_a_radio_group_with_nothing_checked_at_the_first_radio(cx: &mut TestAppContext) {
        let (view, cx) = choices(None, cx);
        assert_eq!(focused(&view, cx), Some(0));
    }

    #[gpui::test]
    fn arrow_keys_move_focus_and_select_skipping_disabled_radios(cx: &mut TestAppContext) {
        let (view, cx) = choices(Some(0), cx);

        cx.simulate_keystrokes("down");
        assert_eq!(
            focused(&view, cx),
            Some(2),
            "the disabled radio is passed over"
        );
        assert_eq!(view.read_with(cx, |choices, _| choices.checked), Some(2));

        cx.simulate_keystrokes("right");
        assert_eq!(
            focused(&view, cx),
            Some(0),
            "past the last radio focus wraps"
        );
        assert_eq!(view.read_with(cx, |choices, _| choices.checked), Some(0));

        cx.simulate_keystrokes("up");
        assert_eq!(view.read_with(cx, |choices, _| choices.checked), Some(2));
    }
}
