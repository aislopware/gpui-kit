//! Roving keyboard focus for composite controls.
//!
//! A composite (tab list, radio group, toggle group, toolbar, accordion) is one
//! stop in the window's Tab order; inside it the arrow keys move focus from item
//! to item, wrapping at either end, Home and End jump to the first and last
//! item, and disabled items are passed over. This is the contract Base UI's
//! `CompositeRoot` gives its Tabs, Radio Group, Toggle Group and Toolbar.
//!
//! Two shapes are offered:
//!
//! - [`RovingFocus`] for a composite that knows its items. The group hands out
//!   one focus handle per item and marks exactly one of them a tab stop: the
//!   focused item while focus is inside, otherwise the active (selected) item,
//!   otherwise the item focused last, otherwise the first enabled one.
//! - The tab-order walk below, for a container of arbitrary focusable children
//!   (the [`crate::Toolbar`]), which only sees the window's rendered tab stops.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gpui::{App, Axis, ElementId, FocusHandle, KeyDownEvent, Keystroke, Window};

/// The arrow keys a roving-focus composite answers to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RovingOrientation {
    /// Left and Right, as in a tab list or toolbar laid out in a row.
    #[default]
    Horizontal,
    /// Up and Down, as in an accordion or a vertical tab list.
    Vertical,
    /// All four arrows, as in a radio group.
    Both,
}

impl From<Axis> for RovingOrientation {
    fn from(axis: Axis) -> Self {
        match axis {
            Axis::Horizontal => Self::Horizontal,
            Axis::Vertical => Self::Vertical,
        }
    }
}

/// Where a key sends focus inside a composite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RovingMove {
    Previous,
    Next,
    First,
    Last,
}

impl RovingMove {
    /// The move a keystroke asks for, or `None` when the composite leaves the
    /// key alone. A modified key (Shift, Control, Option, Command) is never a
    /// roving key, so shortcuts keep working inside the group.
    pub(crate) fn from_keystroke(
        orientation: RovingOrientation,
        keystroke: &Keystroke,
    ) -> Option<Self> {
        if keystroke.modifiers.modified() {
            return None;
        }
        let horizontal = orientation != RovingOrientation::Vertical;
        let vertical = orientation != RovingOrientation::Horizontal;
        match keystroke.key.as_str() {
            "left" if horizontal => Some(Self::Previous),
            "right" if horizontal => Some(Self::Next),
            "up" if vertical => Some(Self::Previous),
            "down" if vertical => Some(Self::Next),
            "home" => Some(Self::First),
            "end" => Some(Self::Last),
            _ => None,
        }
    }
}

/// The item a move lands on, passing over disabled items.
///
/// `from` is the focused item, if any; from none, Next lands on the first
/// enabled item and Previous on the last. With `loop_focus` the walk wraps at
/// either end; without it a move past the end finds nothing. A group whose
/// items are all disabled has no target.
pub(crate) fn roving_target(
    disabled: &[bool],
    from: Option<usize>,
    movement: RovingMove,
    loop_focus: bool,
) -> Option<usize> {
    let len = disabled.len();
    let enabled = |ix: &usize| !disabled[*ix];
    let first = || (0..len).find(enabled);
    let last = || (0..len).rev().find(enabled);
    match (movement, from) {
        (RovingMove::First, _) | (RovingMove::Next, None) => first(),
        (RovingMove::Last, _) | (RovingMove::Previous, None) => last(),
        (RovingMove::Next, Some(from)) => ((from + 1).min(len)..len)
            .find(enabled)
            .or_else(|| loop_focus.then(first).flatten()),
        (RovingMove::Previous, Some(from)) => (0..from.min(len))
            .rev()
            .find(enabled)
            .or_else(|| loop_focus.then(last).flatten()),
    }
}

/// Per-group state that outlives a frame: one focus handle per item and the
/// item focused last.
struct RovingMemory {
    handles: RefCell<Vec<FocusHandle>>,
    last_focused: Cell<Option<usize>>,
}

/// One frame's roving-focus composite over items the group knows.
///
/// Create it in the render that builds the group, give each item the handle
/// from [`Self::focus_handle`] (and its tab stop from [`Self::is_tab_stop`] when
/// the item applies its own), and route the group root's key-down events to
/// [`Self::handle_key_down`]. The handles live in keyed state under the given
/// id, so focus survives rebuilding the group every frame.
///
/// ```ignore
/// let roving = RovingFocus::new("modes-roving", modes.len(), window, cx)
///     .orientation(RovingOrientation::Both)
///     .active(selected)
///     .disabled_items(modes.iter().map(|mode| mode.disabled));
/// ```
#[derive(Clone)]
pub struct RovingFocus {
    memory: Rc<RovingMemory>,
    len: usize,
    disabled: Rc<[bool]>,
    focused: Option<usize>,
    active: Option<usize>,
    orientation: RovingOrientation,
    loop_focus: bool,
    single_tab_stop: bool,
}

impl RovingFocus {
    /// The roving focus of a group of `len` items, keyed by `id`.
    pub fn new(id: impl Into<ElementId>, len: usize, window: &mut Window, cx: &mut App) -> Self {
        let memory = window
            .use_keyed_state(id.into(), cx, |_, _| {
                Rc::new(RovingMemory {
                    handles: RefCell::new(Vec::new()),
                    last_focused: Cell::new(None),
                })
            })
            .read(cx)
            .clone();
        {
            let mut handles = memory.handles.borrow_mut();
            while handles.len() < len {
                handles.push(cx.focus_handle());
            }
        }
        let focused = memory.handles.borrow()[..len]
            .iter()
            .position(|handle| handle.is_focused(window));
        if focused.is_some() {
            memory.last_focused.set(focused);
        }
        Self {
            memory,
            len,
            disabled: vec![false; len].into(),
            focused,
            active: None,
            orientation: RovingOrientation::default(),
            loop_focus: true,
            single_tab_stop: true,
        }
    }

    /// Which arrow keys move focus. Default: [`RovingOrientation::Horizontal`].
    pub fn orientation(mut self, orientation: impl Into<RovingOrientation>) -> Self {
        self.orientation = orientation.into();
        self
    }

    /// Whether moving past either end wraps to the other. Default: `true`.
    pub fn loop_focus(mut self, loop_focus: bool) -> Self {
        self.loop_focus = loop_focus;
        self
    }

    /// Whether the group is a single stop in the Tab order. Default: `true`.
    ///
    /// An accordion leaves every trigger in the Tab order and only adds the
    /// arrow keys, so it turns this off.
    pub fn single_tab_stop(mut self, single_tab_stop: bool) -> Self {
        self.single_tab_stop = single_tab_stop;
        self
    }

    /// The selected item, which takes the group's tab stop while focus is
    /// elsewhere: the selected tab, the checked radio.
    pub fn active(mut self, active: Option<usize>) -> Self {
        self.active = active.filter(|ix| *ix < self.len);
        self
    }

    /// Which items are disabled, in item order. Disabled items never take
    /// focus from the keyboard; items past the end of `disabled` stay enabled.
    pub fn disabled_items(mut self, disabled: impl IntoIterator<Item = bool>) -> Self {
        let mut items = vec![false; self.len];
        for (slot, disabled) in items.iter_mut().zip(disabled) {
            *slot = disabled;
        }
        self.disabled = items.into();
        self
    }

    /// The number of items in the group.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the group has no items.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The item that takes keyboard focus when Tab enters the group.
    pub fn tab_stop_index(&self) -> Option<usize> {
        let enabled = |ix: &usize| *ix < self.len && !self.disabled[*ix];
        self.focused
            .filter(enabled)
            .or(self.active.filter(enabled))
            .or(self.memory.last_focused.get().filter(enabled))
            .or_else(|| (0..self.len).find(enabled))
    }

    /// Whether item `index` is in the window's Tab order.
    pub fn is_tab_stop(&self, index: usize) -> bool {
        if self.single_tab_stop {
            self.tab_stop_index() == Some(index)
        } else {
            index < self.len && !self.disabled[index]
        }
    }

    /// The focus handle of item `index`, a tab stop only when
    /// [`Self::is_tab_stop`] says so.
    ///
    /// # Panics
    ///
    /// When `index` is not below the length the group was created with.
    pub fn focus_handle(&self, index: usize) -> FocusHandle {
        assert!(index < self.len, "roving item {index} of {}", self.len);
        self.memory.handles.borrow()[index]
            .clone()
            .tab_stop(self.is_tab_stop(index))
    }

    /// The item holding keyboard focus, if any.
    pub fn focused_index(&self, window: &Window) -> Option<usize> {
        self.memory.handles.borrow()[..self.len]
            .iter()
            .position(|handle| handle.is_focused(window))
    }

    /// Moves focus for a roving key pressed while an item is focused.
    ///
    /// Returns the item that newly took focus, so a radio group can select it
    /// and a tab list that activates on focus can show it. A roving key on an
    /// item is consumed even when focus has nowhere to go, so the arrows never
    /// scroll the view behind the group; keys pressed while something else in
    /// the group holds focus (a tab's close button) are left alone.
    pub fn handle_key_down(
        &self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<usize> {
        let movement = RovingMove::from_keystroke(self.orientation, &event.keystroke)?;
        let from = self.focused_index(window)?;
        cx.stop_propagation();
        let target = roving_target(&self.disabled, Some(from), movement, self.loop_focus)
            .filter(|target| *target != from)?;
        let handle = self.memory.handles.borrow()[target].clone();
        window.focus(&handle, cx);
        self.memory.last_focused.set(Some(target));
        Some(target)
    }
}

fn step_focus(window: &mut Window, cx: &mut App, forward: bool) {
    if forward {
        window.focus_next(cx);
    } else {
        window.focus_prev(cx);
    }
}

/// Moves focus to the next (or previous) focusable element inside `container`.
///
/// Walk the window's tab order in the asked direction, passing over stops
/// outside the container, and land on the first stop inside it. The tab order
/// wraps around the window, so past the container's last stop the walk comes
/// back in at its first however many stops the rest of the window holds, and a
/// stop that a tab index places between two of the container's own is passed
/// over like any other. GPUI's tab order can cycle short of the whole window,
/// so the walk ends at the first repeated stop and focus goes back where it
/// started: a container with no other reachable stop keeps focus where it is.
pub(crate) fn move_focus_within(
    container: &FocusHandle,
    forward: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(start) = window.focused(cx) else {
        return;
    };

    let mut visited = vec![start.clone()];
    loop {
        step_focus(window, cx, forward);
        let Some(focused) = window.focused(cx) else {
            break;
        };
        if visited.contains(&focused) {
            break;
        }
        if container.contains_focused(window, cx) {
            return;
        }
        visited.push(focused);
    }
    window.focus(&start, cx);
}

/// Moves focus to the first (or, with `last`, the last) focusable element
/// inside `container` in the window's tab order.
///
/// The walk starts from no focus, where GPUI's tab order begins at the
/// window's first stop (or ends at its last), and stops at the first stop
/// inside the container. Nothing is drawn between the steps, so the walk is
/// never seen; when the container holds no reachable stop, focus goes back
/// where it was.
pub(crate) fn focus_edge_within(
    container: &FocusHandle,
    last: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(start) = window.focused(cx) else {
        return;
    };

    window.blur(cx);
    let mut visited = Vec::new();
    loop {
        step_focus(window, cx, !last);
        let Some(focused) = window.focused(cx) else {
            break;
        };
        if container.contains_focused(window, cx) {
            return;
        }
        if visited.contains(&focused) {
            break;
        }
        visited.push(focused);
    }
    window.focus(&start, cx);
}

/// Moves focus out of `container` to the next (or previous) stop past it, as
/// Tab (or Shift-Tab) does from a composite that is one tab stop.
///
/// Stops inside the container are passed over; inside an active focus trap so
/// are stops outside the trap, as `Root` constrains Tab. Returns `false`, with
/// focus back where it was, when no stop outside the container is reachable.
pub(crate) fn leave_container(
    container: &FocusHandle,
    forward: bool,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let Some(start) = window.focused(cx) else {
        return false;
    };
    let trap = crate::active_focus_trap(window, cx);

    let mut visited = vec![start.clone()];
    loop {
        step_focus(window, cx, forward);
        let Some(focused) = window.focused(cx) else {
            break;
        };
        if visited.contains(&focused) {
            break;
        }
        let outside = !container.contains_focused(window, cx);
        let trapped = trap
            .as_ref()
            .is_none_or(|trap| trap.contains_focused(window, cx));
        if outside && trapped {
            return true;
        }
        visited.push(focused);
    }
    window.focus(&start, cx);
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _,
        TestAppContext, VisualTestContext, div, px,
    };

    #[test]
    fn arrows_wrap_around_and_pass_over_disabled_items() {
        let disabled = [false, true, false, false];
        let next = |from| roving_target(&disabled, Some(from), RovingMove::Next, true);
        let previous = |from| roving_target(&disabled, Some(from), RovingMove::Previous, true);

        assert_eq!(next(0), Some(2), "the disabled second item is passed over");
        assert_eq!(
            next(3),
            Some(0),
            "past the last item focus wraps to the first"
        );
        assert_eq!(
            previous(0),
            Some(3),
            "before the first item focus wraps to the last"
        );
        assert_eq!(previous(2), Some(0));
    }

    #[test]
    fn without_looping_a_move_past_the_end_goes_nowhere() {
        let disabled = [false, false, false];
        assert_eq!(
            roving_target(&disabled, Some(2), RovingMove::Next, false),
            None
        );
        assert_eq!(
            roving_target(&disabled, Some(0), RovingMove::Previous, false),
            None
        );
    }

    #[test]
    fn home_and_end_land_on_the_first_and_last_enabled_items() {
        let disabled = [true, false, false, true];
        assert_eq!(
            roving_target(&disabled, Some(2), RovingMove::First, true),
            Some(1)
        );
        assert_eq!(
            roving_target(&disabled, Some(1), RovingMove::Last, true),
            Some(2)
        );
        assert_eq!(
            roving_target(&[true, true], None, RovingMove::First, true),
            None,
            "a group of disabled items has nowhere to go"
        );
    }

    #[test]
    fn every_move_lands_on_an_enabled_item() {
        // Every disabled mask of up to six items, from every start, every move.
        for len in 0..=6usize {
            for mask in 0..(1u32 << len) {
                let disabled: Vec<bool> = (0..len).map(|ix| mask & (1 << ix) != 0).collect();
                let from_options = std::iter::once(None).chain((0..len).map(Some));
                for from in from_options {
                    for movement in [
                        RovingMove::Previous,
                        RovingMove::Next,
                        RovingMove::First,
                        RovingMove::Last,
                    ] {
                        for loop_focus in [false, true] {
                            if let Some(target) =
                                roving_target(&disabled, from, movement, loop_focus)
                            {
                                assert!(!disabled[target], "{disabled:?} {from:?} {movement:?}");
                            } else if loop_focus && disabled.iter().any(|d| !d) {
                                panic!("a looping move found nothing in {disabled:?}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn modified_arrows_are_not_roving_keys() {
        let key = |text: &'static str| Keystroke::parse(text).unwrap();
        let horizontal = RovingOrientation::Horizontal;
        assert_eq!(
            RovingMove::from_keystroke(horizontal, &key("right")),
            Some(RovingMove::Next)
        );
        assert_eq!(RovingMove::from_keystroke(horizontal, &key("down")), None);
        assert_eq!(
            RovingMove::from_keystroke(horizontal, &key("shift-right")),
            None
        );
        assert_eq!(
            RovingMove::from_keystroke(RovingOrientation::Both, &key("up")),
            Some(RovingMove::Previous)
        );
    }

    /// Three items in a roving group; the middle one is selected.
    struct Group {
        disabled: [bool; 3],
        handles: Rc<RefCell<Vec<FocusHandle>>>,
        moves: Rc<RefCell<Vec<usize>>>,
    }

    impl Render for Group {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let roving = RovingFocus::new("group-roving", 3, window, cx)
                .active(Some(1))
                .disabled_items(self.disabled);
            *self.handles.borrow_mut() = (0..3).map(|ix| roving.focus_handle(ix)).collect();
            let moves = self.moves.clone();
            div()
                .id("group")
                .flex()
                .on_key_down(move |event, window, cx| {
                    if let Some(ix) = roving.handle_key_down(event, window, cx) {
                        moves.borrow_mut().push(ix);
                    }
                })
                .children((0..3).map(|ix| {
                    div()
                        .id(("item", ix))
                        .size(px(20.))
                        .track_focus(&self.handles.borrow()[ix])
                }))
        }
    }

    fn group(
        disabled: [bool; 3],
        cx: &mut TestAppContext,
    ) -> (
        Rc<RefCell<Vec<FocusHandle>>>,
        Rc<RefCell<Vec<usize>>>,
        &mut VisualTestContext,
    ) {
        let handles = Rc::new(RefCell::new(Vec::new()));
        let moves = Rc::new(RefCell::new(Vec::new()));
        let (_, cx) = cx.add_window_view({
            let handles = handles.clone();
            let moves = moves.clone();
            move |_, _| Group {
                disabled,
                handles,
                moves,
            }
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        (handles, moves, cx)
    }

    fn focused(
        handles: &Rc<RefCell<Vec<FocusHandle>>>,
        cx: &mut VisualTestContext,
    ) -> Option<usize> {
        cx.update(|window, _| {
            handles
                .borrow()
                .iter()
                .position(|handle| handle.is_focused(window))
        })
    }

    #[gpui::test]
    fn tab_enters_a_group_at_its_selected_item_and_leaves_it_in_one_press(cx: &mut TestAppContext) {
        let (handles, _, cx) = group([false; 3], cx);

        cx.update(|window, cx| window.focus_next(cx));
        assert_eq!(
            focused(&handles, cx),
            Some(1),
            "Tab lands on the selected item"
        );

        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            window.focus_next(cx);
        });
        // The group is the window's only stop, so leaving it comes back round
        // to the same item rather than to a neighbour.
        assert_eq!(
            focused(&handles, cx),
            Some(1),
            "Tab does not stop on another item"
        );
    }

    #[gpui::test]
    fn arrows_move_focus_through_a_group_and_report_where_it_landed(cx: &mut TestAppContext) {
        let (handles, moves, cx) = group([false, false, true], cx);
        cx.update(|window, cx| window.focus_next(cx));

        cx.simulate_keystrokes("right");
        assert_eq!(
            focused(&handles, cx),
            Some(0),
            "the disabled last item is passed over"
        );
        cx.simulate_keystrokes("end");
        assert_eq!(
            focused(&handles, cx),
            Some(1),
            "End lands on the last enabled item"
        );
        cx.simulate_keystrokes("home");
        assert_eq!(focused(&handles, cx), Some(0));
        assert_eq!(*moves.borrow(), vec![0, 1, 0]);
    }

    #[gpui::test]
    fn the_focused_item_becomes_the_groups_only_tab_stop(cx: &mut TestAppContext) {
        let (handles, _, cx) = group([false; 3], cx);
        cx.update(|window, cx| window.focus_next(cx));
        cx.simulate_keystrokes("left");
        cx.update(|window, cx| window.draw(cx).clear(cx));

        let stops: Vec<bool> = handles.borrow().iter().map(|h| h.tab_stop).collect();
        assert_eq!(stops, vec![true, false, false]);
    }
}
