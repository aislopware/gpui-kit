use crate::TestSupportExt as _;
use std::time::Instant;
use std::{cell::RefCell, collections::VecDeque, rc::Rc, time::Duration};

use gpui::{
    Anchor, AnyElement, App, Axis, Div, ElementId, FocusHandle, InteractiveElement, Interactivity,
    IntoElement, LiveRegion as _, MouseMoveEvent, ParentElement, Pixels, Point, RenderOnce, Role,
    Stateful, StatefulInteractiveElement, StyleRefinement, Styled, Window, canvas, div, point,
    prelude::FluentBuilder as _, px,
};

use crate::{
    ElementExt as _, StyledExt as _,
    motion::{Spring, spring},
};

/// Motion tokens used by an unstyled toast stack.
#[derive(Clone, Copy, Debug)]
pub struct ToastMotion {
    /// Time scale of the stack's motion.
    ///
    /// This is read as two different quantities, because the stack sequences
    /// one thing and interpolates another. [`ToastManager`] treats it as a
    /// deadline: a toast is present once it has elapsed. The layout treats it
    /// as a spring response, which is the scale the reflow is felt at rather
    /// than the moment it stops — a toast arriving still nudges the stack for
    /// a little longer than this.
    pub duration: Duration,
    /// Duration before an ending toast is unmounted.
    pub exit_duration: Duration,
    /// Visible distance between collapsed toast layers.
    pub collapsed_peek: Pixels,
    /// Distance between expanded toast items.
    pub expanded_gap: Pixels,
    /// Fractional width reduction for each collapsed layer.
    pub collapsed_scale_step: f32,
    /// Number of layers visible while the stack is collapsed.
    pub collapsed_visible: usize,
}

impl ToastMotion {
    /// Create motion matching the shadcn/Sonner toaster.
    pub fn sonner() -> Self {
        Self {
            duration: Duration::from_millis(400),
            exit_duration: Duration::from_millis(200),
            collapsed_peek: px(14.),
            expanded_gap: px(14.),
            collapsed_scale_step: 0.05,
            collapsed_visible: 3,
        }
    }
}

impl Default for ToastMotion {
    fn default() -> Self {
        Self::sonner()
    }
}

/// Persistent private layout state used by [`ToastStack`].
#[derive(Clone, Debug, Default)]
pub struct ToastStackState {
    heights: Rc<RefCell<std::collections::HashMap<ElementId, Pixels>>>,
    width: Rc<std::cell::Cell<Pixels>>,
    hovered: Rc<std::cell::Cell<bool>>,
    focused: Rc<std::cell::Cell<bool>>,
}

impl ToastStackState {
    /// Return whether interaction has expanded the stack.
    pub fn is_expanded(&self) -> bool {
        self.hovered.get() || self.focused.get()
    }
}

/// Options applied when a toast enters a [`ToastManager`].
#[derive(Clone, Copy, Debug, Default)]
pub struct ToastOptions {
    /// Active time before automatic dismissal; `None` disables auto-hide.
    pub timeout: Option<Duration>,
}

#[derive(Debug)]
struct ManagedToast<I, T> {
    id: I,
    value: T,
    status: ToastTransitionStatus,
    timeout_remaining: Option<Duration>,
    transition_elapsed: Duration,
    last_advance: Instant,
}

/// Changes produced when a toast manager advances its lifecycle clock.
#[derive(Debug)]
pub struct ToastAdvance<I, T> {
    /// Whether a mounted toast changed phase or membership.
    pub changed: bool,
    /// Toast ids whose entry transition completed.
    pub presented: Vec<I>,
    /// Toast ids that entered their ending transition.
    pub ending: Vec<I>,
    /// Toast values removed after their ending transition completed.
    pub removed: Vec<(I, T)>,
}

/// Ordered toast storage, lifecycle, auto-hide, limits, and exit coordination.
#[derive(Debug)]
pub struct ToastManager<I, T> {
    entries: VecDeque<ManagedToast<I, T>>,
    transition_duration: Duration,
    exit_duration: Duration,
}

impl<I, T> ToastManager<I, T> {
    /// Create a manager using the supplied motion duration for enter and exit.
    pub fn new(motion: ToastMotion) -> Self {
        Self {
            entries: VecDeque::new(),
            transition_duration: motion.duration,
            exit_duration: motion.exit_duration,
        }
    }

    /// Return the number of mounted toasts, including ending toasts.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return whether no toast is mounted.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over mounted toast ids, values, and phases in display order.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = (&I, &T, ToastTransitionStatus)> {
        self.entries
            .iter()
            .map(|entry| (&entry.id, &entry.value, entry.status))
    }

    /// Iterate over newest visible active toasts plus ending toasts.
    pub fn visible(&self, limit: usize) -> impl Iterator<Item = (&I, &T, ToastTransitionStatus)> {
        let first = self
            .entries
            .iter()
            .filter(|entry| entry.status != ToastTransitionStatus::Ending)
            .count()
            .saturating_sub(limit);
        let mut active_index = 0usize;
        self.entries.iter().filter_map(move |entry| {
            let visible = if entry.status == ToastTransitionStatus::Ending {
                true
            } else {
                let keep = active_index >= first;
                active_index += 1;
                keep
            };
            visible.then_some((&entry.id, &entry.value, entry.status))
        })
    }

    /// Return a mounted toast value by id.
    pub fn get(&self, id: &I) -> Option<&T>
    where
        I: Eq,
    {
        self.entries
            .iter()
            .find_map(|entry| (&entry.id == id).then_some(&entry.value))
    }
}

impl<I: Clone + Eq, T> ToastManager<I, T> {
    /// Add a newest toast, replacing an existing toast with the same id.
    pub fn push(&mut self, id: I, value: T, options: ToastOptions, now: Instant) -> Option<T> {
        let replaced = self
            .entries
            .iter()
            .position(|entry| entry.id == id)
            .and_then(|index| self.entries.remove(index))
            .map(|entry| entry.value);
        self.entries.push_back(ManagedToast {
            id,
            value,
            status: ToastTransitionStatus::Starting,
            timeout_remaining: options.timeout,
            transition_elapsed: Duration::ZERO,
            last_advance: now,
        });
        replaced
    }

    /// Begin a toast's exit transition, returning whether its state changed.
    pub fn dismiss(&mut self, id: &I, now: Instant) -> bool {
        let Some(entry) = self.entries.iter_mut().find(|entry| &entry.id == id) else {
            return false;
        };
        if entry.status == ToastTransitionStatus::Ending {
            return false;
        }
        entry.status = ToastTransitionStatus::Ending;
        entry.transition_elapsed = Duration::ZERO;
        entry.last_advance = now;
        true
    }

    /// Begin the exit transition for every active toast.
    pub fn dismiss_all(&mut self, now: Instant) -> Vec<I> {
        let mut changed = Vec::new();
        for entry in &mut self.entries {
            if entry.status != ToastTransitionStatus::Ending {
                entry.status = ToastTransitionStatus::Ending;
                entry.transition_elapsed = Duration::ZERO;
                entry.last_advance = now;
                changed.push(entry.id.clone());
            }
        }
        changed
    }

    /// Advance lifecycle time; active timers pause while `paused` is true.
    pub fn advance(&mut self, now: Instant, paused: bool) -> ToastAdvance<I, T> {
        let mut ending = Vec::new();
        let mut presented = Vec::new();
        let mut changed = false;
        for entry in &mut self.entries {
            let delta = now.saturating_duration_since(entry.last_advance);
            entry.last_advance = now;
            match entry.status {
                ToastTransitionStatus::Starting => {
                    entry.transition_elapsed += delta;
                    if entry.transition_elapsed >= self.transition_duration {
                        entry.status = ToastTransitionStatus::Present;
                        entry.transition_elapsed = Duration::ZERO;
                        presented.push(entry.id.clone());
                        changed = true;
                    }
                }
                ToastTransitionStatus::Present if !paused => {
                    if let Some(remaining) = &mut entry.timeout_remaining {
                        *remaining = remaining.saturating_sub(delta);
                        if remaining.is_zero() {
                            entry.status = ToastTransitionStatus::Ending;
                            entry.transition_elapsed = Duration::ZERO;
                            ending.push(entry.id.clone());
                            changed = true;
                        }
                    }
                }
                ToastTransitionStatus::Ending => entry.transition_elapsed += delta,
                ToastTransitionStatus::Present => {}
            }
        }
        let mut removed = Vec::new();
        let mut index = 0;
        while index < self.entries.len() {
            if self.entries[index].status == ToastTransitionStatus::Ending
                && self.entries[index].transition_elapsed >= self.exit_duration
            {
                let entry = self.entries.remove(index).expect("toast index is valid");
                removed.push((entry.id, entry.value));
                changed = true;
            } else {
                index += 1;
            }
        }
        ToastAdvance {
            changed,
            presented,
            ending,
            removed,
        }
    }
}

/// A deep toast-stack element that owns measurement, overlap, and expansion motion.
///
/// The stack is a landmark region, as Base UI's `Toast.Viewport`, so
/// assistive technology can list it and jump to it. Name it with
/// `aria_label`; the label is the application's copy.
#[derive(IntoElement)]
pub struct ToastStack {
    id: ElementId,
    base: crate::ObservedElement<Stateful<Div>>,
    style: StyleRefinement,
    state: ToastStackState,
    motion: ToastMotion,
    placement: Anchor,
    focus_handle: Option<FocusHandle>,
    children: Vec<(ElementId, AnyElement)>,
}

impl ToastStack {
    /// Create a toast stack with Base UI-compatible motion.
    pub fn new(id: impl Into<ElementId>, state: ToastStackState) -> Self {
        let id = id.into();
        Self {
            base: div().id(id.clone()).test_support(),
            id,
            style: StyleRefinement::default(),
            state,
            motion: ToastMotion::sonner(),
            placement: Anchor::TopRight,
            focus_handle: None,
            children: Vec::new(),
        }
    }

    /// Add a stably keyed toast item to the stack.
    pub fn item(mut self, id: impl Into<ElementId>, child: impl IntoElement) -> Self {
        self.children.push((id.into(), child.into_any_element()));
        self
    }

    /// Set the stack motion tokens.
    pub fn motion(mut self, motion: ToastMotion) -> Self {
        self.motion = motion;
        self
    }

    /// Set the viewport edge used to anchor stack geometry.
    pub fn placement(mut self, placement: Anchor) -> Self {
        self.placement = placement;
        self
    }

    /// Set the focus scope that expands the stack and pauses auto-hide timers.
    pub fn focus_handle(mut self, focus_handle: FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle);
        self
    }
}

impl Styled for ToastStack {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for ToastStack {
    fn extend(&mut self, children: impl IntoIterator<Item = AnyElement>) {
        self.children
            .extend(children.into_iter().enumerate().map(|(index, child)| {
                (
                    ElementId::NamedInteger("toast-stack-child".into(), index as u64),
                    child,
                )
            }));
    }
}

impl InteractiveElement for ToastStack {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }

    fn track_focus(mut self, handle: &gpui::FocusHandle) -> Self {
        self.base = self.base.track_focus(handle);
        self
    }
}

impl StatefulInteractiveElement for ToastStack {}

fn stack_geometry(
    heights: &[Pixels],
    gap: Pixels,
    peek: Pixels,
    anchored_bottom: bool,
) -> (Pixels, Pixels, Vec<(Pixels, Pixels)>) {
    let count = heights.len();
    let expanded_height = heights
        .iter()
        .copied()
        .fold(px(0.), |sum, height| sum + height)
        + gap * count.saturating_sub(1) as f32;
    let front_height = heights.last().copied().unwrap_or(px(0.));
    let collapsed_height = heights
        .iter()
        .enumerate()
        .map(|(index, height)| {
            let rank = count - 1 - index;
            *height + peek * rank as f32
        })
        .fold(
            front_height + peek * count.saturating_sub(1) as f32,
            Pixels::max,
        );
    let offsets = heights
        .iter()
        .enumerate()
        .map(|(index, height)| {
            let rank = count - 1 - index;
            let newer_height = heights[(index + 1)..]
                .iter()
                .copied()
                .fold(px(0.), |sum, height| sum + height);
            let expanded = if anchored_bottom {
                expanded_height - newer_height - gap * rank as f32 - *height
            } else {
                newer_height + gap * rank as f32
            };
            let collapsed = if anchored_bottom {
                collapsed_height - *height - peek * rank as f32
            } else {
                peek * rank as f32
            };
            (collapsed, expanded)
        })
        .collect();
    (collapsed_height, expanded_height, offsets)
}

impl RenderOnce for ToastStack {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focused = self
            .focus_handle
            .as_ref()
            .is_some_and(|handle| handle.contains_focused(window, cx));
        self.state.focused.set(focused);
        let expanded = self.state.is_expanded();
        let keys = self
            .children
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        self.state
            .heights
            .borrow_mut()
            .retain(|id, _| keys.contains(id));
        let measured_by_id = self.state.heights.borrow().clone();
        let heights = keys
            .iter()
            .map(|id| measured_by_id.get(id).copied().unwrap_or(px(0.)))
            .collect::<Vec<_>>();
        let measured = self.state.heights.clone();
        let peek = self.motion.collapsed_peek;
        let gap = self.motion.expanded_gap;
        let scale_step = self.motion.collapsed_scale_step;
        let collapsed_visible = self.motion.collapsed_visible.max(1);
        let stack_width = self.state.width.get();
        let count = self.children.len();
        let anchored_bottom = matches!(
            self.placement,
            Anchor::BottomLeft | Anchor::BottomCenter | Anchor::BottomRight
        );
        let (collapsed_height, expanded_height, offsets) = stack_geometry(
            &heights[..count.min(heights.len())],
            gap,
            peek,
            anchored_bottom,
        );
        // The whole stack reflows every time a toast arrives or leaves, so each
        // layer is sprung: one retargeted mid-move carries its velocity into the
        // new layout instead of restarting. Critically damped, because a height
        // or an opacity that overshoots its target reads as a glitch. Geometry
        // settles in pixels, so its tolerance is coarser than the fade's.
        //
        // A bottom-anchored item's position is composed from two of these — the
        // stack height and the item's own offset — and the stack height only
        // acquires its real target once the new toast has been measured in
        // prepaint, a frame after the offsets know theirs. Two springs sharing a
        // config stay proportional to each other only while they also share a
        // start, so that one frame of skew lets the composed position pass its
        // settled value by a fraction of a pixel before arriving. Both springs
        // snap on settling, so it is a transient, not a resting error.
        let geometry = Spring::new(self.motion.duration).with_epsilon(0.1);
        let fade = Spring::new(self.motion.duration);
        let stack_height = spring(
            (self.id.clone(), "height"),
            if expanded {
                expanded_height
            } else {
                collapsed_height
            },
            geometry,
            window,
            cx,
        );
        let items = self
            .children
            .into_iter()
            .enumerate()
            .map(move |(index, (item_id, child))| {
                let (collapsed_offset, expanded_offset) =
                    offsets.get(index).copied().unwrap_or((px(0.), px(0.)));
                let measured = measured.clone();
                let measured_id = item_id.clone();
                let rank = count.saturating_sub(1 + index);
                let target_offset = if expanded {
                    expanded_offset
                } else {
                    collapsed_offset
                };
                let offset = spring(
                    (item_id.clone(), "offset"),
                    target_offset,
                    geometry,
                    window,
                    cx,
                );
                let target_inset = if expanded {
                    px(0.)
                } else {
                    stack_width * (scale_step * rank.min(collapsed_visible - 1) as f32 / 2.)
                };
                let inset = spring(
                    (item_id.clone(), "inset"),
                    target_inset,
                    geometry,
                    window,
                    cx,
                );
                let opacity = spring(
                    (item_id.clone(), "visibility"),
                    if expanded || rank < collapsed_visible {
                        1.
                    } else {
                        0.
                    },
                    fade,
                    window,
                    cx,
                );
                div()
                    .id(item_id.clone())
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .top(offset)
                    .left(inset)
                    .right(inset)
                    .opacity(opacity)
                    .when(!expanded && rank >= collapsed_visible, |this| {
                        this.invisible()
                    })
                    .on_prepaint(move |bounds, _, cx| {
                        let mut heights = measured.borrow_mut();
                        if heights.get(&measured_id).copied() != Some(bounds.size.height) {
                            heights.insert(measured_id.clone(), bounds.size.height);
                            cx.refresh_windows();
                        }
                    })
                    .child(child)
            });

        let hovered_state = self.state.hovered.clone();
        let measured_width = self.state.width.clone();
        self.base
            .role(Role::Region)
            .relative()
            .h(stack_height)
            .when_some(self.focus_handle, |this, handle| this.track_focus(&handle))
            .child(
                canvas(
                    |_, _, _| {},
                    move |mut bounds, _, window, cx| {
                        if measured_width.replace(bounds.size.width) != bounds.size.width {
                            cx.refresh_windows();
                        }
                        if !expanded {
                            if anchored_bottom {
                                bounds.origin.y += stack_height - collapsed_height;
                            }
                            bounds.size.height = collapsed_height;
                        }
                        let hovered_state = hovered_state.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, _, _, cx| {
                            let hovered = bounds.contains(&event.position);
                            if hovered_state.replace(hovered) != hovered {
                                cx.refresh_windows();
                            }
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
            .children(items)
            .refine_style(&self.style)
    }
}

/// The lifecycle phase exposed by a toast root to application-owned presentation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ToastTransitionStatus {
    /// The toast has just been added and may run its enter transition.
    #[default]
    Starting,
    /// The toast is fully present.
    Present,
    /// The toast is closing and remains mounted until its exit transition completes.
    Ending,
}

/// How urgently assistive technology announces a toast, as Base UI's
/// `priority`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToastPriority {
    /// A status, announced after what is being said.
    #[default]
    Low,
    /// An alert, announced at once: errors and what needs action now.
    High,
}

/// A direction a toast can be swiped away in, as Base UI's `swipeDirection`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SwipeDirection {
    /// Toward the top of the window.
    Up,
    /// Toward the bottom of the window.
    Down,
    /// Toward the left of the window.
    Left,
    /// Toward the right of the window.
    Right,
}

impl SwipeDirection {
    /// The directions that carry a toast in a stack at `placement` off the
    /// window: toward the edge the stack sits against, and toward the nearer
    /// side, which is right for a centered stack.
    ///
    /// This is Base UI's default of down and right, said for every placement:
    /// a swipe only dismisses when it throws the toast out of the way it came,
    /// never across the content it covers.
    pub fn off_screen(placement: Anchor) -> Vec<Self> {
        let vertical = match placement {
            Anchor::TopLeft | Anchor::TopCenter | Anchor::TopRight => Some(Self::Up),
            Anchor::BottomLeft | Anchor::BottomCenter | Anchor::BottomRight => Some(Self::Down),
            Anchor::LeftCenter | Anchor::RightCenter => None,
        };
        let horizontal = match placement {
            Anchor::TopLeft | Anchor::BottomLeft | Anchor::LeftCenter => Self::Left,
            _ => Self::Right,
        };
        vertical.into_iter().chain([horizontal]).collect()
    }

    /// The direction a displacement of `distance` along `axis` points in, if
    /// it points anywhere.
    fn along(axis: Axis, distance: f32) -> Option<Self> {
        match axis {
            _ if distance == 0. => None,
            Axis::Horizontal if distance > 0. => Some(Self::Right),
            Axis::Horizontal => Some(Self::Left),
            Axis::Vertical if distance > 0. => Some(Self::Down),
            Axis::Vertical => Some(Self::Up),
        }
    }

    /// `1` for the directions the window's coordinates grow in, `-1` for the
    /// others.
    fn sign(self) -> f32 {
        match self {
            Self::Down | Self::Right => 1.,
            Self::Up | Self::Left => -1.,
        }
    }
}

/// How far a swipe travels before it commits to an axis, in pixels, so the
/// tremor of a press is not read as a direction.
const SWIPE_SLOP: f32 = 3.;
/// The distance at which a pull in a direction the toast cannot leave by
/// stops giving, in pixels.
const SWIPE_RESISTANCE: f32 = 24.;
/// The release speed that dismisses a toast short of the threshold, in pixels
/// per millisecond.
const SWIPE_FLICK_VELOCITY: f32 = 0.4;
/// The least travel a flick needs, so a fast twitch of a press is no swipe.
const SWIPE_FLICK_DISTANCE: f32 = 8.;
/// How far back the release speed looks: the speed at the moment of letting
/// go, not the average of the whole gesture.
const SWIPE_VELOCITY_WINDOW: Duration = Duration::from_millis(100);

/// The movement a pull of `distance` gives in a direction the toast cannot
/// leave by: a little at first, stiffening toward [`SWIPE_RESISTANCE`], so the
/// toast answers the pull and still reads as refusing it.
fn resist(distance: f32) -> f32 {
    distance / (1. + distance.abs() / SWIPE_RESISTANCE)
}

/// Swipe-to-dismiss for one toast, as Base UI's `Toast.Root` swipe.
///
/// It reads positions and times and answers with the offset to draw the toast
/// at and, on release, whether the toast leaves. The application feeds it a
/// press-drag or a trackpad's two-finger scroll alike and owns the drawing.
///
/// A gesture locks to the axis it first moves along. Along that axis the toast
/// follows the pointer in a direction it may leave by and resists in the
/// other; it leaves when released at least [`ToastSwipe::THRESHOLD`] out, or
/// flicked out faster than a deliberate drag. Times come from the caller, so a
/// test clock decides a flick exactly as the platform's does.
#[derive(Clone, Debug, Default)]
pub struct ToastSwipe {
    directions: Vec<SwipeDirection>,
    gesture: Option<SwipeGesture>,
    offset: Point<Pixels>,
    moved: bool,
}

#[derive(Clone, Debug)]
struct SwipeGesture {
    origin: Point<Pixels>,
    axis: Option<Axis>,
    /// Displacements along the locked axis, oldest first, kept for
    /// [`SWIPE_VELOCITY_WINDOW`] to measure the release speed.
    samples: VecDeque<(f32, Instant)>,
}

impl ToastSwipe {
    /// The distance a toast must be released at to leave, as Base UI's 40 px.
    pub const THRESHOLD: Pixels = px(40.);

    /// Create a swipe that dismisses only in `directions`.
    pub fn new(directions: impl IntoIterator<Item = SwipeDirection>) -> Self {
        Self {
            directions: directions.into_iter().collect(),
            ..Self::default()
        }
    }

    /// Start a gesture with the pointer at `position`.
    pub fn begin(&mut self, position: Point<Pixels>, now: Instant) {
        // The press is the first sample, so a flick made in a single move
        // still has a speed.
        self.gesture = Some(SwipeGesture {
            origin: position,
            axis: None,
            samples: VecDeque::from([(0., now)]),
        });
        self.offset = Point::default();
        self.moved = false;
    }

    /// Follow the pointer to `position`.
    pub fn move_to(&mut self, position: Point<Pixels>, now: Instant) {
        let Some(gesture) = &mut self.gesture else {
            return;
        };
        let dx = (position.x - gesture.origin.x).as_f32();
        let dy = (position.y - gesture.origin.y).as_f32();
        let axis = match gesture.axis {
            Some(axis) => axis,
            None if dx.abs().max(dy.abs()) < SWIPE_SLOP => return,
            None => {
                let axis = if dx.abs() >= dy.abs() {
                    Axis::Horizontal
                } else {
                    Axis::Vertical
                };
                gesture.axis = Some(axis);
                self.moved = true;
                axis
            }
        };
        let distance = match axis {
            Axis::Horizontal => dx,
            Axis::Vertical => dy,
        };
        gesture.samples.push_back((distance, now));
        Self::forget_before(&mut gesture.samples, now);
        let shown = match SwipeDirection::along(axis, distance) {
            Some(direction) if self.directions.contains(&direction) => distance,
            _ => resist(distance),
        };
        self.offset = match axis {
            Axis::Horizontal => point(px(shown), px(0.)),
            Axis::Vertical => point(px(0.), px(shown)),
        };
    }

    /// End the gesture, returning the direction the toast leaves in.
    ///
    /// A toast that leaves keeps its offset, so its exit continues from where
    /// the pointer let go; one that stays is offset back to rest, and the
    /// application springs it there.
    pub fn release(&mut self, now: Instant) -> Option<SwipeDirection> {
        let mut gesture = self.gesture.take()?;
        let leaving = gesture.axis.and_then(|axis| {
            let &(distance, _) = gesture.samples.back()?;
            let direction = SwipeDirection::along(axis, distance)
                .filter(|direction| self.directions.contains(direction))?;
            Self::forget_before(&mut gesture.samples, now);
            let velocity = match (gesture.samples.front(), gesture.samples.back()) {
                (Some(&(from, at)), Some(&(to, _))) => {
                    let elapsed = now.saturating_duration_since(at).as_secs_f32() * 1000.;
                    if elapsed > 0. {
                        (to - from) / elapsed
                    } else {
                        0.
                    }
                }
                _ => 0.,
            };
            let out = distance * direction.sign();
            let flicked =
                velocity * direction.sign() >= SWIPE_FLICK_VELOCITY && out >= SWIPE_FLICK_DISTANCE;
            (out >= Self::THRESHOLD.as_f32() || flicked).then_some(direction)
        });
        if leaving.is_none() {
            self.offset = Point::default();
        }
        leaving
    }

    /// Abandon the gesture, as when the system takes the touch: the toast
    /// goes back to rest and does not leave.
    pub fn cancel(&mut self) {
        self.gesture = None;
        self.offset = Point::default();
    }

    /// The offset to draw the toast at, from where it rests.
    pub fn offset(&self) -> Point<Pixels> {
        self.offset
    }

    /// Whether a gesture is in progress.
    pub fn is_swiping(&self) -> bool {
        self.gesture.is_some()
    }

    /// Whether the latest gesture moved far enough to be a swipe rather than
    /// a press, so the release is not also taken as a click.
    pub fn has_moved(&self) -> bool {
        self.moved
    }

    /// Drop the samples older than [`SWIPE_VELOCITY_WINDOW`] before `now`.
    /// With none left inside it, the pointer was at rest and the release
    /// speed is zero.
    fn forget_before(samples: &mut VecDeque<(f32, Instant)>, now: Instant) {
        while samples
            .front()
            .is_some_and(|&(_, at)| now.saturating_duration_since(at) > SWIPE_VELOCITY_WINDOW)
        {
            samples.pop_front();
        }
    }
}

/// An unstyled semantic toast root. Applications own all presentation and motion.
#[derive(IntoElement)]
pub struct Toast {
    base: crate::ObservedElement<Stateful<Div>>,
    style: StyleRefinement,
    transition_status: ToastTransitionStatus,
    priority: ToastPriority,
    children: Vec<AnyElement>,
}

impl Toast {
    /// Create an unstyled semantic toast in the starting transition phase.
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(id).test_support(),
            style: StyleRefinement::default(),
            transition_status: ToastTransitionStatus::Starting,
            priority: ToastPriority::Low,
            children: Vec::new(),
        }
    }

    /// Set how urgently the toast is announced; [`ToastPriority::Low`] by
    /// default.
    pub fn priority(mut self, priority: ToastPriority) -> Self {
        self.priority = priority;
        self
    }

    /// Set the lifecycle phase used by application-owned toast presentation.
    pub fn transition_status(mut self, status: ToastTransitionStatus) -> Self {
        self.transition_status = status;
        self
    }

    /// Return the lifecycle phase used by application-owned toast presentation.
    pub fn status(&self) -> ToastTransitionStatus {
        self.transition_status
    }
}

impl Styled for Toast {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Toast {
    fn extend(&mut self, children: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(children);
    }
}

impl InteractiveElement for Toast {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }

    fn track_focus(mut self, handle: &gpui::FocusHandle) -> Self {
        self.base = self.base.track_focus(handle);
        self
    }
}

impl StatefulInteractiveElement for Toast {}

impl RenderOnce for Toast {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let (role, live) = match self.priority {
            ToastPriority::Low => (Role::Status, gpui::accesskit::Live::Polite),
            ToastPriority::High => (Role::Alert, gpui::accesskit::Live::Assertive),
        };
        self.base
            .role(role)
            .aria_live(live)
            .children(self.children)
            .refine_style(&self.style)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Element as _, accesskit, point};

    #[test]
    fn manager_pauses_timeout_and_removes_only_after_exit() {
        let motion = ToastMotion::sonner();
        let start = Instant::now();
        let mut manager = ToastManager::new(motion);
        manager.push(
            "a",
            1,
            ToastOptions {
                timeout: Some(Duration::from_secs(5)),
            },
            start,
        );
        manager.advance(start + motion.duration, false);
        manager.advance(start + motion.duration + Duration::from_secs(4), true);
        assert_eq!(
            manager.iter().next().unwrap().2,
            ToastTransitionStatus::Present
        );
        let ending = manager.advance(start + motion.duration + Duration::from_secs(9), false);
        assert_eq!(ending.ending, vec!["a"]);
        assert!(ending.removed.is_empty());
        let removed = manager.advance(
            start + motion.duration + motion.exit_duration + Duration::from_secs(9),
            false,
        );
        assert_eq!(removed.removed, vec![("a", 1)]);
    }

    #[test]
    fn manager_limit_keeps_ending_toasts_mounted() {
        let now = Instant::now();
        let mut manager = ToastManager::new(ToastMotion::sonner());
        for id in ["a", "b", "c"] {
            manager.push(id, id, ToastOptions::default(), now);
        }
        manager.dismiss(&"a", now);
        assert_eq!(
            manager.visible(1).map(|(id, _, _)| *id).collect::<Vec<_>>(),
            vec!["a", "c"]
        );
    }

    #[test]
    fn manager_replaces_duplicate_ids_as_the_newest_toast() {
        let now = Instant::now();
        let mut manager = ToastManager::new(ToastMotion::sonner());
        manager.push("a", 1, ToastOptions::default(), now);
        manager.push("b", 2, ToastOptions::default(), now);
        assert_eq!(manager.push("a", 3, ToastOptions::default(), now), Some(1));
        assert_eq!(
            manager
                .iter()
                .map(|(id, value, _)| (*id, *value))
                .collect::<Vec<_>>(),
            vec![("b", 2), ("a", 3)]
        );
    }

    #[test]
    fn manager_resets_clock_when_reused_after_becoming_empty() {
        let motion = ToastMotion::sonner();
        let start = Instant::now();
        let mut manager = ToastManager::new(motion);
        manager.push("old", 1, ToastOptions::default(), start);
        manager.dismiss(&"old", start);
        manager.advance(start + motion.exit_duration, false);
        let later = start + Duration::from_secs(60);
        manager.push("new", 2, ToastOptions::default(), later);
        manager.advance(later + Duration::from_millis(50), false);
        assert_eq!(
            manager.iter().next().unwrap().2,
            ToastTransitionStatus::Starting
        );
    }

    #[test]
    fn newly_pushed_toast_does_not_inherit_existing_entry_clock() {
        let motion = ToastMotion::sonner();
        let start = Instant::now();
        let mut manager = ToastManager::new(motion);
        manager.push("old", 1, ToastOptions::default(), start);
        let later = start + Duration::from_secs(60);
        manager.push("new", 2, ToastOptions::default(), later);
        manager.advance(later + Duration::from_millis(50), false);
        assert_eq!(
            manager
                .iter()
                .find(|(id, _, _)| **id == "new")
                .map(|(_, _, status)| status),
            Some(ToastTransitionStatus::Starting)
        );
    }

    #[test]
    fn stack_geometry_anchors_newest_item_and_supports_variable_heights() {
        let heights = [px(40.), px(60.), px(80.)];
        let (collapsed, expanded, top) = stack_geometry(&heights, px(12.), px(12.), false);
        assert_eq!(collapsed, px(104.));
        assert_eq!(expanded, px(204.));
        assert_eq!(
            top,
            vec![(px(24.), px(164.)), (px(12.), px(92.)), (px(0.), px(0.))]
        );

        let (_, _, bottom) = stack_geometry(&heights, px(12.), px(12.), true);
        assert_eq!(
            bottom,
            vec![(px(40.), px(0.)), (px(32.), px(52.)), (px(24.), px(124.))]
        );

        let tall_behind = [px(180.), px(60.)];
        let (collapsed, _, top) = stack_geometry(&tall_behind, px(14.), px(14.), false);
        assert_eq!(collapsed, px(194.));
        assert_eq!(top[0].0, px(14.));
        assert!(top[0].0 + tall_behind[0] <= collapsed);
        assert_eq!(top[0].0 - top[1].0, px(14.));

        let (_, _, bottom) = stack_geometry(&tall_behind, px(14.), px(14.), true);
        assert!(bottom[0].0 >= px(0.));
        assert!(bottom[0].0 + tall_behind[0] <= collapsed);
        assert_eq!(
            (bottom[1].0 + tall_behind[1]) - (bottom[0].0 + tall_behind[0]),
            px(14.)
        );
    }

    #[gpui::test]
    fn a_toast_is_a_polite_status_and_a_high_priority_one_an_assertive_alert(
        cx: &mut gpui::TestAppContext,
    ) {
        let window = cx.add_empty_window();
        window.update(|window, cx| {
            let mut node = accesskit::Node::new(Role::Unknown);
            Toast::new("toast")
                .render(window, cx)
                .into_element()
                .write_a11y_info(&mut node);
            assert_eq!(node.live(), Some(accesskit::Live::Polite));

            let mut node = accesskit::Node::new(Role::Unknown);
            Toast::new("alert")
                .priority(ToastPriority::High)
                .render(window, cx)
                .into_element()
                .write_a11y_info(&mut node);
            assert_eq!(node.live(), Some(accesskit::Live::Assertive));
        });
    }

    #[gpui::test]
    fn the_stack_is_a_labelled_landmark_region(cx: &mut gpui::TestAppContext) {
        struct Harness {
            state: ToastStackState,
            seen: Rc<RefCell<Option<(Option<Role>, Option<String>)>>>,
        }
        impl gpui::Render for Harness {
            fn render(
                &mut self,
                window: &mut Window,
                cx: &mut gpui::Context<Self>,
            ) -> impl IntoElement {
                let stack = ToastStack::new("stack", self.state.clone())
                    .aria_label("Notifications")
                    .item("first", div().h(px(40.)))
                    .render(window, cx)
                    .into_element();
                let mut node = accesskit::Node::new(Role::Unknown);
                stack.write_a11y_info(&mut node);
                *self.seen.borrow_mut() =
                    Some((stack.a11y_role(), node.label().map(str::to_owned)));
                stack
            }
        }

        let seen = Rc::new(RefCell::new(None));
        let (_, cx) = cx.add_window_view({
            let seen = seen.clone();
            move |_, _| Harness {
                state: ToastStackState::default(),
                seen,
            }
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(
            seen.borrow().clone(),
            Some((Some(Role::Region), Some("Notifications".to_owned())))
        );
    }

    const START: Point<Pixels> = point(px(100.), px(100.));

    fn at(dx: f32, dy: f32) -> Point<Pixels> {
        point(START.x + px(dx), START.y + px(dy))
    }

    #[test]
    fn off_screen_directions_follow_the_edges_the_stack_sits_against() {
        use SwipeDirection::*;
        assert_eq!(SwipeDirection::off_screen(Anchor::TopRight), [Up, Right]);
        assert_eq!(
            SwipeDirection::off_screen(Anchor::BottomRight),
            [Down, Right]
        );
        assert_eq!(SwipeDirection::off_screen(Anchor::BottomLeft), [Down, Left]);
        assert_eq!(SwipeDirection::off_screen(Anchor::TopCenter), [Up, Right]);
        assert_eq!(SwipeDirection::off_screen(Anchor::LeftCenter), [Left]);
        assert_eq!(SwipeDirection::off_screen(Anchor::RightCenter), [Right]);
    }

    #[test]
    fn a_swipe_released_at_the_threshold_leaves_and_one_short_of_it_rests() {
        let now = Instant::now();
        let mut swipe = ToastSwipe::new(SwipeDirection::off_screen(Anchor::TopRight));

        swipe.begin(START, now);
        swipe.move_to(at(39., 0.), now);
        assert!(swipe.is_swiping());
        assert_eq!(swipe.offset(), point(px(39.), px(0.)));
        assert_eq!(swipe.release(now), None);
        assert!(!swipe.is_swiping());
        assert_eq!(swipe.offset(), Point::default());

        swipe.begin(START, now);
        swipe.move_to(at(0., -40.), now);
        assert_eq!(swipe.release(now), Some(SwipeDirection::Up));
        // The exit continues from where the pointer let go.
        assert_eq!(swipe.offset(), point(px(0.), px(-40.)));
    }

    #[test]
    fn a_swipe_resists_and_never_leaves_in_a_direction_it_may_not() {
        let now = Instant::now();
        let mut swipe = ToastSwipe::new(SwipeDirection::off_screen(Anchor::TopRight));
        swipe.begin(START, now);
        let mut previous = px(0.);
        for distance in [10., 40., 200., 2000.] {
            swipe.move_to(at(-distance, 0.), now);
            let shown = swipe.offset().x;
            assert!(shown < previous, "a harder pull still gives a little");
            assert!(shown > px(-SWIPE_RESISTANCE), "{shown:?} at {distance}");
            assert!(-shown.as_f32() < distance);
            previous = shown;
        }
        assert_eq!(swipe.release(now), None);
        assert_eq!(swipe.offset(), Point::default());
    }

    #[test]
    fn a_swipe_locks_to_the_axis_it_first_moves_along() {
        let now = Instant::now();
        let mut swipe = ToastSwipe::new([SwipeDirection::Down, SwipeDirection::Right]);
        swipe.begin(START, now);

        // Within the slop a press is still a press.
        swipe.move_to(at(2., 1.), now);
        assert_eq!(swipe.offset(), Point::default());
        assert!(!swipe.has_moved());

        swipe.move_to(at(10., 4.), now);
        assert!(swipe.has_moved());
        assert_eq!(swipe.offset(), point(px(10.), px(0.)));

        // A later turn downward does not move it off the horizontal.
        swipe.move_to(at(10., 80.), now);
        assert_eq!(swipe.offset(), point(px(10.), px(0.)));
        assert_eq!(swipe.release(now), None);
        assert!(swipe.has_moved(), "the release is not also a click");

        swipe.cancel();
        swipe.begin(START, now);
        swipe.move_to(at(0., 30.), now);
        swipe.cancel();
        assert!(!swipe.is_swiping());
        assert_eq!(swipe.offset(), Point::default());
    }

    #[test]
    fn a_flick_leaves_short_of_the_threshold_but_a_paused_or_tiny_one_does_not() {
        let start = Instant::now();
        let ms = Duration::from_millis;
        let mut swipe = ToastSwipe::new([SwipeDirection::Right]);

        // 15 px in 20 ms is 0.75 px/ms.
        swipe.begin(START, start);
        swipe.move_to(at(15., 0.), start + ms(20));
        assert_eq!(swipe.release(start + ms(20)), Some(SwipeDirection::Right));

        // The same drag held still before letting go has no speed left.
        swipe.begin(START, start);
        swipe.move_to(at(15., 0.), start + ms(20));
        assert_eq!(swipe.release(start + ms(300)), None);

        // Only the last stretch counts: a slow drag that ends in a fast one
        // is a flick.
        swipe.begin(START, start);
        swipe.move_to(at(5., 0.), start + ms(400));
        swipe.move_to(at(6., 0.), start + ms(800));
        swipe.move_to(at(20., 0.), start + ms(820));
        assert_eq!(swipe.release(start + ms(820)), Some(SwipeDirection::Right));

        // A twitch is fast but too short to be meant.
        swipe.begin(START, start);
        swipe.move_to(at(6., 0.), start + ms(4));
        assert_eq!(swipe.release(start + ms(4)), None);

        // A flick back the other way is refused like any pull there.
        swipe.begin(START, start);
        swipe.move_to(at(-30., 0.), start + ms(20));
        assert_eq!(swipe.release(start + ms(20)), None);
    }

    #[gpui::test]
    fn stack_bootstraps_measurement_without_zero_height_clipping(cx: &mut gpui::TestAppContext) {
        struct Harness {
            state: ToastStackState,
        }
        impl gpui::Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
                ToastStack::new("stack", self.state.clone())
                    .w(px(300.))
                    .item("first", div().h(px(80.)).child("Visible"))
            }
        }

        let (view, cx) = cx.add_window_view(|_, _| Harness {
            state: ToastStackState::default(),
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        view.read_with(cx, |view, _| {
            assert_eq!(
                view.state
                    .heights
                    .borrow()
                    .get(&ElementId::from("first"))
                    .copied(),
                Some(px(80.))
            );
        });
    }

    #[gpui::test]
    fn stack_expands_for_hover_and_focus(cx: &mut gpui::TestAppContext) {
        struct Harness {
            state: ToastStackState,
            focus: FocusHandle,
        }
        impl gpui::Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
                ToastStack::new("stack", self.state.clone())
                    .focus_handle(self.focus.clone())
                    .w(px(300.))
                    .item("first", div().h(px(80.)))
            }
        }

        let (view, cx) = cx.add_window_view(|_, cx| Harness {
            state: ToastStackState::default(),
            focus: cx.focus_handle().tab_stop(true),
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));

        cx.simulate_mouse_move(point(px(10.), px(10.)), None, gpui::Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        view.read_with(cx, |view, _| assert!(view.state.is_expanded()));

        cx.simulate_mouse_move(point(px(400.), px(400.)), None, gpui::Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        view.read_with(cx, |view, _| assert!(!view.state.is_expanded()));

        cx.simulate_mouse_move(point(px(10.), px(10.)), None, gpui::Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        view.read_with(cx, |view, _| assert!(view.state.is_expanded()));

        cx.simulate_mouse_move(point(px(400.), px(400.)), None, gpui::Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        view.read_with(cx, |view, _| assert!(!view.state.is_expanded()));

        let focus = view.read_with(cx, |view, _| view.focus.clone());
        cx.update(|window, cx| {
            focus.focus(window, cx);
            window.draw(cx).clear(cx);
        });
        view.read_with(cx, |view, _| assert!(view.state.is_expanded()));
    }

    #[gpui::test]
    fn keyed_stack_reflow_moves_from_the_current_visual_position(cx: &mut gpui::TestAppContext) {
        struct Harness {
            state: ToastStackState,
            show_second: bool,
        }
        impl gpui::Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
                ToastStack::new("stack", self.state.clone())
                    .w(px(300.))
                    .item(
                        "first",
                        div().debug_selector(|| "first-toast".into()).h(px(40.)),
                    )
                    .when(self.show_second, |stack| {
                        stack.item(
                            "second",
                            div().debug_selector(|| "second-toast".into()).h(px(80.)),
                        )
                    })
            }
        }

        let (view, cx) = cx.add_window_view(|_, _| Harness {
            state: ToastStackState::default(),
            show_second: false,
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            window.draw(cx).clear(cx);
        });
        let initial_y = cx.debug_bounds("first-toast").unwrap().origin.y;

        view.update(cx, |view, cx| {
            view.show_second = true;
            cx.notify();
        });
        cx.run_until_parked();
        let first_reflow_y = cx.debug_bounds("first-toast").unwrap().origin.y;
        assert_eq!(first_reflow_y, initial_y);

        cx.executor().advance_clock(Duration::from_millis(200));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let middle_y = cx.debug_bounds("first-toast").unwrap().origin.y;
        assert!(middle_y > initial_y);
        assert!(middle_y < initial_y + px(14.));

        cx.executor().advance_clock(Duration::from_millis(200));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(
            cx.debug_bounds("first-toast").unwrap().origin.y,
            initial_y + px(14.)
        );
        assert!(
            cx.debug_bounds("first-toast").unwrap().size.width
                < cx.debug_bounds("second-toast").unwrap().size.width
        );
    }

    #[gpui::test]
    fn bottom_stack_reflow_moves_up_without_an_opposite_direction_jump(
        cx: &mut gpui::TestAppContext,
    ) {
        struct Harness {
            state: ToastStackState,
            show_second: bool,
        }
        impl gpui::Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
                div().relative().size(px(300.)).child(
                    ToastStack::new("bottom-stack", self.state.clone())
                        .placement(Anchor::BottomRight)
                        .absolute()
                        .bottom_0()
                        .w(px(300.))
                        .item(
                            "bottom-first",
                            div()
                                .debug_selector(|| "bottom-first-toast".into())
                                .h(px(40.)),
                        )
                        .when(self.show_second, |stack| {
                            stack.item("bottom-second", div().h(px(80.)))
                        }),
                )
            }
        }

        let (view, cx) = cx.add_window_view(|_, _| Harness {
            state: ToastStackState::default(),
            show_second: false,
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            window.draw(cx).clear(cx);
        });
        // A bottom-anchored item's position is composed from two springs — the
        // stack height and the item's own offset — and the stack height only
        // acquires its real target once the toast has been measured in prepaint.
        // Both the baseline and the final reading are taken settled, so neither
        // carries the fraction of a pixel that separates them mid-flight.
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let initial_y = cx.debug_bounds("bottom-first-toast").unwrap().origin.y;

        view.update(cx, |view, cx| {
            view.show_second = true;
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(
            cx.debug_bounds("bottom-first-toast").unwrap().origin.y,
            initial_y
        );

        cx.executor().advance_clock(Duration::from_millis(200));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let middle_y = cx.debug_bounds("bottom-first-toast").unwrap().origin.y;
        assert!(middle_y < initial_y);
        assert!(
            middle_y >= initial_y - px(14.),
            "middle={middle_y:?}, initial={initial_y:?}"
        );

        cx.executor().advance_clock(Duration::from_secs(1));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(
            cx.debug_bounds("bottom-first-toast").unwrap().origin.y,
            initial_y - px(14.)
        );
    }
}
