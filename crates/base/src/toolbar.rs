use gpui::{
    AnyElement, App, Axis, Div, ElementId, FocusHandle, InteractiveElement, Interactivity,
    IntoElement, KeyDownEvent, ParentElement, RenderOnce, Role, SharedString, Stateful,
    StatefulInteractiveElement, StyleRefinement, Styled, Window, accesskit, div,
    prelude::FluentBuilder as _,
};
use smallvec::SmallVec;

use crate::{
    StyledExt as _, TestSupportExt as _,
    roving_focus::{
        RovingMove, RovingOrientation, focus_edge_within, leave_container, move_focus_within,
    },
};

/// An unstyled container that groups a set of controls and owns roving
/// keyboard focus among them.
///
/// This is the behavior primitive behind a styled toolbar. The container
/// exposes `Toolbar` semantics to assistive technology (`Role::Toolbar` with
/// an orientation) and moves focus between its focusable descendants with the
/// arrow keys, so applications do not have to reimplement the roving-focus
/// contract per toolbar. It works with any focusable children — buttons,
/// menu triggers, inputs — because traversal walks the rendered tab stops
/// and constrains them to this subtree, mirroring how `Root` constrains
/// focus-trap cycling.
///
/// Keyboard contract:
///
/// - Left and Right move focus to the previous or next focusable descendant,
///   wrapping around at either end (the same default as Base UI's `loopFocus`).
///   A vertical toolbar ([`Toolbar::axis`]) uses Up and Down instead and
///   reports a vertical orientation.
/// - Home and End move focus to the first and last focusable descendant.
/// - The toolbar is one stop in the window's Tab order: Tab and Shift-Tab from
///   any item leave the toolbar instead of visiting its other items. Tab from
///   before the toolbar enters it at its first item, Shift-Tab from after it
///   at its last.
/// - When `disabled`, none of these keys do anything special. Hosted
///   controls must be disabled by their owner; the flag only suppresses the
///   toolbar's own navigation.
///
/// The container itself never takes focus. Disabled controls do not track
/// focus, so the walk passes over them.
///
/// An input hosted inside the toolbar keeps its own arrow-key behavior: text
/// inputs consume the arrow keys for caret movement before the toolbar sees
/// them. Place inputs at the trailing end of a horizontal toolbar, as the
/// Base UI Toolbar recommends.
#[derive(IntoElement)]
pub struct Toolbar {
    id: ElementId,
    base: crate::ObservedElement<Stateful<Div>>,
    style: StyleRefinement,
    disabled: bool,
    axis: Axis,
    children: SmallVec<[AnyElement; 4]>,
}

impl Toolbar {
    pub fn new(id: impl Into<ElementId>) -> Self {
        let id = id.into();
        Self {
            base: div().id(id.clone()).test_support(),
            style: StyleRefinement::default(),
            disabled: false,
            axis: Axis::Horizontal,
            children: SmallVec::new(),
            id,
        }
    }

    /// Disables the toolbar's own keyboard navigation. Hosted controls are
    /// not automatically disabled.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The direction the items run: horizontal (Left/Right, the default) or
    /// vertical (Up/Down). Assistive technology hears the same orientation.
    pub fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }
}

fn handle_key_down(
    disabled: bool,
    axis: Axis,
    container: &FocusHandle,
    event: &KeyDownEvent,
    window: &mut Window,
    cx: &mut App,
) {
    if disabled {
        return;
    }

    let orientation = RovingOrientation::from(axis);
    match RovingMove::from_keystroke(orientation, &event.keystroke) {
        Some(RovingMove::Previous) => move_focus_within(container, false, window, cx),
        Some(RovingMove::Next) => move_focus_within(container, true, window, cx),
        Some(RovingMove::First) => focus_edge_within(container, false, window, cx),
        Some(RovingMove::Last) => focus_edge_within(container, true, window, cx),
        None => return,
    }
    cx.stop_propagation();
}

/// Tab (or Shift-Tab) from inside the toolbar skips its remaining items: the
/// whole toolbar is one stop in the window's Tab order.
fn handle_tab(
    disabled: bool,
    container: &FocusHandle,
    forward: bool,
    window: &mut Window,
    cx: &mut App,
) {
    if disabled || !leave_container(container, forward, window, cx) {
        cx.propagate();
    }
}

impl Styled for Toolbar {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Toolbar {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl InteractiveElement for Toolbar {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for Toolbar {}

impl RenderOnce for Toolbar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            base,
            style,
            disabled,
            axis,
            children,
            ..
        } = self;

        // The handle must survive across frames so containment checks and the
        // key handler refer to the same dispatch-tree node; button.rs uses
        // the same keyed-state pattern for its own focus handle.
        let focus_handle = window
            .use_keyed_state(self.id.clone(), cx, |_, cx| {
                cx.focus_handle().tab_stop(false)
            })
            .read(cx)
            .clone();
        let key_handler = {
            let focus_handle = focus_handle.clone();
            move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
                handle_key_down(disabled, axis, &focus_handle, event, window, cx);
            }
        };
        let tab_handler = {
            let focus_handle = focus_handle.clone();
            move |_: &crate::root::Tab, window: &mut Window, cx: &mut App| {
                handle_tab(disabled, &focus_handle, true, window, cx);
            }
        };
        let tab_prev_handler = {
            let focus_handle = focus_handle.clone();
            move |_: &crate::root::TabPrev, window: &mut Window, cx: &mut App| {
                handle_tab(disabled, &focus_handle, false, window, cx);
            }
        };

        base.track_focus(&focus_handle)
            .role(Role::Toolbar)
            .aria_orientation(match axis {
                Axis::Horizontal => accesskit::Orientation::Horizontal,
                Axis::Vertical => accesskit::Orientation::Vertical,
            })
            .on_key_down(key_handler)
            .on_action(tab_handler)
            .on_action(tab_prev_handler)
            // The handle exists for containment checks only. Without this, a
            // press on blank space or non-focusable content would focus the
            // toolbar itself and take focus away from the rest of the window.
            // Focusable items handle the press first and keep their focus.
            .on_any_mouse_down(|_, window, _| window.prevent_default())
            .children(children)
            .refine_style(&style)
    }
}

/// A semantic subgroup of items within a [`Toolbar`].
///
/// The group carries no behavior or styling of its own: the surrounding
/// toolbar's roving arrow-key focus traverses its items exactly like the
/// toolbar's direct children, because containment follows the element tree.
/// Its value is structure — assistive technology announces the group and its
/// accessible name, so a run of related controls reads as one unit ("Undo",
/// "Redo" inside a "History" group).
///
/// Unlike Base UI's `Toolbar.Group`, the group cannot disable its children.
/// That API propagates through React context into Base UI's own button
/// primitives; GPUI composition offers no equivalent for arbitrary children,
/// and the platform a11y layer exposes no disabled state for a container
/// node. Disabling the hosted controls is the group owner's job.
#[derive(IntoElement)]
pub struct ToolbarGroup {
    base: Stateful<Div>,
    style: StyleRefinement,
    label: Option<SharedString>,
    children: SmallVec<[AnyElement; 4]>,
}

impl ToolbarGroup {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(id.into()),
            style: StyleRefinement::default(),
            label: None,
            children: SmallVec::new(),
        }
    }

    /// Sets the accessible name announced for the group, e.g. "History".
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }
}

impl Styled for ToolbarGroup {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for ToolbarGroup {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl InteractiveElement for ToolbarGroup {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for ToolbarGroup {}

impl RenderOnce for ToolbarGroup {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        // A group is one inline segment of the bar, so its children flow
        // along the row and center on the bar's cross axis; the same neutral
        // geometry `Tab` applies. Spacing between items is the caller's
        // (matching the bar's own gap).
        self.base
            .flex()
            .items_center()
            .role(Role::Group)
            .when_some(self.label, |this, label| this.aria_label(label))
            .children(self.children)
            .refine_style(&self.style)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_toolbar_builder() {
        let toolbar = Toolbar::new("toolbar").disabled(true).child(div());

        assert!(toolbar.disabled);
        assert_eq!(toolbar.children.len(), 1);
    }

    #[test]
    fn test_toolbar_defaults() {
        let toolbar = Toolbar::new("toolbar");

        assert!(!toolbar.disabled);
        assert_eq!(toolbar.axis, Axis::Horizontal);
        assert!(toolbar.children.is_empty());
        assert_eq!(
            Toolbar::new("toolbar").axis(Axis::Vertical).axis,
            Axis::Vertical
        );
    }

    #[test]
    fn test_toolbar_group_builder() {
        let group = ToolbarGroup::new("history-group")
            .label("History")
            .child(div())
            .child(div());

        assert_eq!(group.label.as_deref(), Some("History"));
        assert_eq!(group.children.len(), 2);
    }

    #[cfg(test)]
    mod behavior {
        use super::*;
        use gpui::{
            Context, Element as _, FocusHandle, Modifiers, Render, TestAppContext,
            VisualTestContext, canvas, point, px,
        };
        use std::sync::{Arc, Mutex};

        struct NavHarness {
            items: [FocusHandle; 3],
        }

        impl Render for NavHarness {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let [first, second, third] = &self.items;
                Toolbar::new("nav-toolbar")
                    .child(div().id("first").size(px(20.)).track_focus(first))
                    .child(div().id("second").size(px(20.)).track_focus(second))
                    .child(div().id("third").size(px(20.)).track_focus(third))
            }
        }

        fn harness(cx: &mut TestAppContext) -> ([FocusHandle; 3], &mut VisualTestContext) {
            let (state, cx) = cx.add_window_view(move |window, cx| {
                let items = [
                    cx.focus_handle().tab_stop(true),
                    cx.focus_handle().tab_stop(true),
                    cx.focus_handle().tab_stop(true),
                ];
                items[0].focus(window, cx);
                NavHarness { items }
            });
            let items = state.read_with(cx, |harness, _| harness.items.clone());
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });
            (items, cx)
        }

        fn assert_focused(cx: &mut VisualTestContext, expected: &FocusHandle, label: &str) {
            cx.update(|window, _| {
                assert!(
                    expected.is_focused(window),
                    "expected {label} to be focused"
                );
            });
        }

        #[gpui::test]
        fn arrow_keys_rove_focus_across_items(cx: &mut gpui::TestAppContext) {
            let ([first, second, third], cx) = harness(cx);

            cx.simulate_keystrokes("right");
            assert_focused(cx, &second, "second");
            cx.simulate_keystrokes("right");
            assert_focused(cx, &third, "third");

            // Wrapping: past the last item, focus returns to the first.
            cx.simulate_keystrokes("right");
            assert_focused(cx, &first, "first (wrapped)");

            cx.simulate_keystrokes("left");
            assert_focused(cx, &third, "third (wrapped back)");
        }

        #[gpui::test]
        fn a_vertical_toolbar_roves_with_up_and_down(cx: &mut TestAppContext) {
            struct Vertical([FocusHandle; 3]);

            impl Render for Vertical {
                fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                    let [first, second, third] = &self.0;
                    Toolbar::new("vertical-toolbar")
                        .axis(Axis::Vertical)
                        .child(div().id("first").size(px(20.)).track_focus(first))
                        .child(div().id("second").size(px(20.)).track_focus(second))
                        .child(div().id("third").size(px(20.)).track_focus(third))
                }
            }

            let (state, cx) = cx.add_window_view(|window, cx| {
                let items = [(); 3].map(|_| cx.focus_handle().tab_stop(true));
                items[0].focus(window, cx);
                Vertical(items)
            });
            let [first, second, third] = state.read_with(cx, |harness, _| harness.0.clone());
            cx.update(|window, cx| window.draw(cx).clear(cx));

            cx.simulate_keystrokes("right");
            assert_focused(cx, &first, "first (Right is not a vertical key)");
            cx.simulate_keystrokes("down");
            assert_focused(cx, &second, "second");
            cx.simulate_keystrokes("down down");
            assert_focused(cx, &first, "first (wrapped)");
            cx.simulate_keystrokes("up");
            assert_focused(cx, &third, "third (wrapped back)");
        }

        #[gpui::test]
        fn home_and_end_jump_to_the_first_and_last_item(cx: &mut TestAppContext) {
            let ([first, _, third], cx) = harness(cx);

            cx.simulate_keystrokes("end");
            assert_focused(cx, &third, "third (End)");
            cx.simulate_keystrokes("home");
            assert_focused(cx, &first, "first (Home)");
            cx.simulate_keystrokes("right end");
            assert_focused(cx, &third, "third (End from the second)");
        }

        /// A toolbar between two page stops.
        struct Framed {
            before: FocusHandle,
            items: [FocusHandle; 3],
            after: FocusHandle,
        }

        impl Render for Framed {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let [first, second, third] = &self.items;
                div()
                    .flex()
                    .child(div().id("before").size(px(20.)).track_focus(&self.before))
                    .child(
                        Toolbar::new("framed-toolbar")
                            .child(div().id("first").size(px(20.)).track_focus(first))
                            .child(div().id("second").size(px(20.)).track_focus(second))
                            .child(div().id("third").size(px(20.)).track_focus(third)),
                    )
                    .child(div().id("after").size(px(20.)).track_focus(&self.after))
            }
        }

        fn framed(cx: &mut TestAppContext) -> (Framed, &mut VisualTestContext) {
            // Tab looks for an enclosing focus trap, as it does in an app.
            cx.update(crate::init);
            let (state, cx) = cx.add_window_view(|window, cx| {
                let stop = |cx: &mut Context<Framed>| cx.focus_handle().tab_stop(true);
                let page = Framed {
                    before: stop(cx),
                    items: [stop(cx), stop(cx), stop(cx)],
                    after: stop(cx),
                };
                page.items[1].focus(window, cx);
                page
            });
            let page = state.read_with(cx, |page, _| Framed {
                before: page.before.clone(),
                items: page.items.clone(),
                after: page.after.clone(),
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            (page, cx)
        }

        #[gpui::test]
        fn tab_from_any_item_leaves_the_toolbar_in_one_press(cx: &mut TestAppContext) {
            let (page, cx) = framed(cx);

            cx.dispatch_action(crate::root::Tab);
            assert_focused(cx, &page.after, "the stop after the toolbar (Tab)");

            cx.update(|window, cx| page.items[1].focus(window, cx));
            cx.dispatch_action(crate::root::TabPrev);
            assert_focused(cx, &page.before, "the stop before the toolbar (Shift-Tab)");
        }

        #[gpui::test]
        fn modified_arrows_leave_focus_where_it_is(cx: &mut TestAppContext) {
            let (page, cx) = framed(cx);

            cx.simulate_keystrokes("shift-right cmd-left");
            assert_focused(cx, &page.items[1], "the second item");
        }

        #[gpui::test]
        fn arrow_keys_wrap_inside_the_toolbar_among_many_page_stops(cx: &mut TestAppContext) {
            // More than the hundred hops the search used to give up after.
            const STOPS: usize = 120;

            struct BusyPage {
                before: Vec<FocusHandle>,
                items: [FocusHandle; 3],
                after: Vec<FocusHandle>,
            }

            impl Render for BusyPage {
                fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                    let [first, second, third] = &self.items;
                    div()
                        .flex()
                        .flex_col()
                        .children(self.before.iter().enumerate().map(|(at, handle)| {
                            div().id(("before", at)).size(px(1.)).track_focus(handle)
                        }))
                        .child(
                            Toolbar::new("busy-toolbar")
                                .child(div().id("first").size(px(20.)).track_focus(first))
                                .child(div().id("second").size(px(20.)).track_focus(second))
                                .child(div().id("third").size(px(20.)).track_focus(third)),
                        )
                        .children(self.after.iter().enumerate().map(|(at, handle)| {
                            div().id(("after", at)).size(px(1.)).track_focus(handle)
                        }))
                }
            }

            let (state, cx) = cx.add_window_view(|window, cx| {
                let stop = |cx: &mut Context<BusyPage>| cx.focus_handle().tab_stop(true);
                let page = BusyPage {
                    before: (0..STOPS).map(|_| stop(cx)).collect(),
                    items: [stop(cx), stop(cx), stop(cx)],
                    after: (0..STOPS).map(|_| stop(cx)).collect(),
                };
                page.items[0].focus(window, cx);
                page
            });
            let [first, second, third] = state.read_with(cx, |page, _| page.items.clone());
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });

            cx.simulate_keystrokes("left");
            assert_focused(cx, &third, "third (Left from the first)");
            cx.simulate_keystrokes("right");
            assert_focused(cx, &first, "first (Right from the last)");
            cx.simulate_keystrokes("right");
            assert_focused(cx, &second, "second (one step right)");
            cx.simulate_keystrokes("left");
            assert_focused(cx, &first, "first (one step left)");
        }

        #[gpui::test]
        fn arrow_keys_wrap_across_more_than_a_hundred_items(cx: &mut TestAppContext) {
            const ITEMS: usize = 120;

            struct LargeToolbar {
                outside: FocusHandle,
                items: Vec<FocusHandle>,
            }

            impl Render for LargeToolbar {
                fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                    div()
                        .child(div().id("outside").size(px(1.)).track_focus(&self.outside))
                        .child(Toolbar::new("large-toolbar").children(
                            self.items.iter().enumerate().map(|(at, handle)| {
                                div().id(("item", at)).size(px(1.)).track_focus(handle)
                            }),
                        ))
                }
            }

            let (state, cx) = cx.add_window_view(|window, cx| {
                let page = LargeToolbar {
                    outside: cx.focus_handle().tab_stop(true),
                    items: (0..ITEMS)
                        .map(|_| cx.focus_handle().tab_stop(true))
                        .collect(),
                };
                page.items[0].focus(window, cx);
                page
            });
            let items = state.read_with(cx, |page, _| page.items.clone());
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });

            cx.simulate_keystrokes("left");
            assert_focused(cx, &items[ITEMS - 1], "the last item (Left from the first)");
            cx.simulate_keystrokes("right");
            assert_focused(cx, &items[0], "the first item (Right from the last)");
        }

        #[gpui::test]
        fn arrow_keys_pass_over_stops_ordered_between_the_items(cx: &mut TestAppContext) {
            struct InterleavedPage {
                items: [FocusHandle; 3],
                outside: [FocusHandle; 2],
            }

            impl Render for InterleavedPage {
                fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                    let [first, second, third] = &self.items;
                    let [before, after] = &self.outside;
                    div()
                        .child(
                            Toolbar::new("interleaved-toolbar")
                                .child(div().id("first").size(px(20.)).track_focus(first))
                                .child(div().id("second").size(px(20.)).track_focus(second))
                                .child(div().id("third").size(px(20.)).track_focus(third)),
                        )
                        .child(div().id("before").size(px(20.)).track_focus(before))
                        .child(div().id("after").size(px(20.)).track_focus(after))
                }
            }

            // The tab order runs first, before, second, after, third: each page stop's
            // tab index puts it between two of the toolbar's items.
            let (state, cx) = cx.add_window_view(|window, cx| {
                let stop = |index, cx: &mut Context<InterleavedPage>| {
                    cx.focus_handle().tab_stop(true).tab_index(index)
                };
                let page = InterleavedPage {
                    items: [stop(1, cx), stop(3, cx), stop(5, cx)],
                    outside: [stop(2, cx), stop(4, cx)],
                };
                page.items[0].focus(window, cx);
                page
            });
            let [first, second, third] = state.read_with(cx, |page, _| page.items.clone());
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });

            cx.simulate_keystrokes("right");
            assert_focused(cx, &second, "second (Right from the first)");
            cx.simulate_keystrokes("right");
            assert_focused(cx, &third, "third (Right from the second)");
            cx.simulate_keystrokes("right");
            assert_focused(cx, &first, "first (Right from the last)");
            cx.simulate_keystrokes("left");
            assert_focused(cx, &third, "third (Left from the first)");
        }

        #[gpui::test]
        fn arrow_keys_keep_focus_when_no_other_item_is_a_tab_stop(cx: &mut TestAppContext) {
            struct Unreachable {
                item: FocusHandle,
                outside: [FocusHandle; 2],
            }

            impl Render for Unreachable {
                fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                    let [before, after] = &self.outside;
                    div()
                        .child(div().id("before").size(px(20.)).track_focus(before))
                        .child(
                            Toolbar::new("unreachable-toolbar")
                                .child(div().id("item").size(px(20.)).track_focus(&self.item)),
                        )
                        .child(div().id("after").size(px(20.)).track_focus(after))
                }
            }

            // The focused item is not a tab stop, so the walk never comes back to
            // it: only the page's own stops repeat.
            let (state, cx) = cx.add_window_view(|window, cx| {
                let page = Unreachable {
                    item: cx.focus_handle(),
                    outside: [
                        cx.focus_handle().tab_stop(true),
                        cx.focus_handle().tab_stop(true),
                    ],
                };
                page.item.focus(window, cx);
                page
            });
            let item = state.read_with(cx, |page, _| page.item.clone());
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });

            cx.simulate_keystrokes("right left");
            assert_focused(cx, &item, "the item (no other stop in the toolbar)");
        }

        #[gpui::test]
        fn toolbar_with_a_single_item_keeps_focus_on_it(cx: &mut gpui::TestAppContext) {
            struct SingleHarness {
                item: FocusHandle,
            }

            impl Render for SingleHarness {
                fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                    let item = &self.item;
                    Toolbar::new("single-toolbar")
                        .child(div().id("only").size(px(20.)).track_focus(item))
                }
            }

            let (state, cx) = cx.add_window_view(|window, cx| {
                let item = cx.focus_handle().tab_stop(true);
                item.focus(window, cx);
                SingleHarness { item }
            });
            let item = state.read_with(cx, |harness, _| harness.item.clone());
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });

            cx.simulate_keystrokes("right left right");
            assert_focused(cx, &item, "the only item");
        }

        #[gpui::test]
        fn snapshot_reports_the_toolbar_role_label_and_focus(cx: &mut gpui::TestAppContext) {
            struct Labelled(FocusHandle);

            impl Render for Labelled {
                fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                    Toolbar::new("formatting")
                        .aria_label("Formatting")
                        .child(div().id("bold").size(px(20.)).track_focus(&self.0))
                }
            }

            let (_, cx) = cx.add_window_view(|window, cx| {
                let item = cx.focus_handle().tab_stop(true);
                item.focus(window, cx);
                Labelled(item)
            });
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                let snapshot = crate::test_support::find(window, &[], &"formatting".into())
                    .expect("the toolbar is observed");
                assert_eq!(snapshot.role(), Some(Role::Toolbar));
                assert_eq!(snapshot.label(), Some("Formatting"));
                // The toolbar's handle is not a tab stop; it reports focus
                // while one of its items holds it.
                assert_eq!(snapshot.focused(), Some(true));
            });
        }

        #[gpui::test]
        fn group_exposes_group_role_and_accessible_name(cx: &mut gpui::TestAppContext) {
            type Captured = Arc<Mutex<Option<accesskit::Node>>>;

            struct Probe(Captured);

            impl Render for Probe {
                fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                    let captured = self.0.clone();
                    canvas(
                        move |_, window, cx| {
                            let mut node = accesskit::Node::new(Role::Group);
                            ToolbarGroup::new("history")
                                .label("History")
                                .child(div().size(px(20.)))
                                .render(window, cx)
                                .into_element()
                                .write_a11y_info(&mut node);
                            *captured.lock().unwrap() = Some(node);
                        },
                        |_, _, _, _| {},
                    )
                }
            }

            let captured: Captured = Arc::new(Mutex::new(None));
            let result = captured.clone();
            let (_, cx) = cx.add_window_view(move |_, _| Probe(captured));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let node = result.lock().unwrap().take().unwrap();

            assert_eq!(node.role(), Role::Group);
            assert_eq!(node.label(), Some("History"));
        }

        #[gpui::test]
        fn disabled_toolbar_ignores_arrow_keys(cx: &mut gpui::TestAppContext) {
            struct DisabledHarness {
                items: [FocusHandle; 2],
            }

            impl Render for DisabledHarness {
                fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                    let [first, second] = &self.items;
                    Toolbar::new("disabled-toolbar")
                        .disabled(true)
                        .child(div().id("first").size(px(20.)).track_focus(first))
                        .child(div().id("second").size(px(20.)).track_focus(second))
                }
            }

            let (state, cx) = cx.add_window_view(|window, cx| {
                let items = [
                    cx.focus_handle().tab_stop(true),
                    cx.focus_handle().tab_stop(true),
                ];
                items[0].focus(window, cx);
                DisabledHarness { items }
            });
            let items = state.read_with(cx, |harness, _| harness.items.clone());
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });

            cx.simulate_keystrokes("right");
            assert_focused(cx, &items[0], "first (disabled toolbar)");
        }

        #[gpui::test]
        fn clicking_blank_toolbar_space_keeps_focus_outside(cx: &mut gpui::TestAppContext) {
            struct ClickHarness {
                outside: FocusHandle,
                item: FocusHandle,
            }

            impl Render for ClickHarness {
                fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                    div()
                        .child(div().id("outside").size(px(20.)).track_focus(&self.outside))
                        .child(
                            Toolbar::new("click-toolbar")
                                .flex()
                                .w(px(100.))
                                .h(px(20.))
                                .child(div().id("item").size(px(20.)).track_focus(&self.item)),
                        )
                }
            }

            let (state, cx) = cx.add_window_view(|window, cx| {
                let outside = cx.focus_handle().tab_stop(true);
                outside.focus(window, cx);
                ClickHarness {
                    outside,
                    item: cx.focus_handle().tab_stop(true),
                }
            });
            let (outside, item) = state.read_with(cx, |harness, _| {
                (harness.outside.clone(), harness.item.clone())
            });
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });

            // Blank space to the right of the item: focus stays where it was.
            cx.simulate_click(point(px(80.), px(30.)), Modifiers::default());
            assert_focused(cx, &outside, "outside (blank toolbar click)");

            // A focusable item still takes focus when clicked.
            cx.simulate_click(point(px(10.), px(30.)), Modifiers::default());
            assert_focused(cx, &item, "item (clicked)");
        }
    }
}
