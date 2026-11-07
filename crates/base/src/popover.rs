use std::{rc::Rc, time::Duration};

use gpui::{
    Anchor, AnyElement, App, Context, DismissEvent, ElementId, EventEmitter, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, KeyBinding, MouseButton, ParentElement as _,
    Render, RenderOnce, Role, StatefulInteractiveElement as _, StyleRefinement, Styled,
    Subscription, Window, div, prelude::FluentBuilder as _,
};

use crate::{
    DeferredPopover, GlobalState, InitialFocus, Popup, Presence, PresencePhase, ResolvedPosition,
    Selectable, StyledExt as _, Transition,
    actions::{Cancel, Confirm},
    hover_open::{self, HoverHost, HoverOpen},
};

/// How long the pointer rests on the trigger of a popover that opens on hover
/// before it opens, Base UI's default.
const HOVER_DELAY: Duration = Duration::from_millis(300);

const CONTEXT: &str = "Popover";
const CONTENT_CONTEXT: &str = "PopoverContent";

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Cancel, Some(CONTEXT)),
        KeyBinding::new("enter", Confirm { secondary: false }, Some(CONTEXT)),
        KeyBinding::new("space", Confirm { secondary: false }, Some(CONTEXT)),
        KeyBinding::new("escape", Cancel, Some(CONTENT_CONTEXT)),
        // The content is still a descendant of the trigger's dispatch node.
        // Leave activation to its controls, not the trigger or an outer dialog.
        // More deeply scoped input bindings retain precedence.
        KeyBinding::new("enter", gpui::NoAction, Some(CONTENT_CONTEXT)),
        KeyBinding::new("space", gpui::NoAction, Some(CONTENT_CONTEXT)),
    ]);
}

type OpenChangeHandler = Rc<dyn Fn(&bool, &mut Window, &mut App)>;

/// State and focus lifecycle for an unstyled popover.
///
/// Applications own trigger/content construction, positioning, appearance, and
/// animation. This state owns the controlled open lifecycle, dismissal, focus
/// capture and restoration, and deferred-popup registration.
pub struct PopoverState {
    focus_handle: FocusHandle,
    tracked_focus_handle: Option<FocusHandle>,
    previous_focus_handle: Option<FocusHandle>,
    open: bool,
    on_open_change: Option<OpenChangeHandler>,
    dismiss_subscription: Option<Subscription>,
    /// While open: focus leaving the popover closes it, as Base UI's
    /// non-modal popover does.
    focus_out_subscription: Option<Subscription>,
    /// Moves the focus to the content's first control once it is drawn, for
    /// a popover whose content takes the focus itself.
    initial_focus: InitialFocus,
    /// Held while open, so that a state collected without being closed — this
    /// lives in element state — takes its registration with it.
    deferred_context: Option<DeferredPopover>,
    /// Whether the pointer resting on the trigger opens the popover.
    open_on_hover: bool,
    hover: HoverOpen,
    /// Opened by the pointer and not pressed since: leaving closes it, and
    /// it has not taken the focus.
    opened_by_hover: bool,
    /// While opened by the pointer: Escape, from any focus in its window,
    /// closes it.
    escape: Option<Subscription>,
    /// How far the content is through its motion, as of this render.
    motion_progress: f32,
    /// Closed, with the content still drawn for its exit motion.
    exiting: bool,
}

impl PopoverState {
    pub fn new(default_open: bool, cx: &mut App) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            tracked_focus_handle: None,
            previous_focus_handle: None,
            open: default_open,
            on_open_change: None,
            dismiss_subscription: None,
            focus_out_subscription: None,
            initial_focus: InitialFocus::Idle,
            deferred_context: None,
            open_on_hover: false,
            hover: HoverOpen::new(HOVER_DELAY, Duration::ZERO),
            opened_by_hover: false,
            escape: None,
            motion_progress: if default_open { 1. } else { 0. },
            exiting: false,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// How far the content is through its [`Popover::motion`], for the
    /// content to draw from: 0 closed, 1 open, between them while it enters
    /// or exits. Without motion it is 1 whenever the content is drawn.
    pub fn motion_progress(&self) -> f32 {
        self.motion_progress
    }

    /// Whether the popover has closed and its content is drawn only for its
    /// exit motion, as Base UI marks a popup with `data-ending-style`.
    pub fn is_exiting(&self) -> bool {
        self.exiting
    }

    pub fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.toggle_open(window, cx);
        }
    }

    pub fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            self.toggle_open(window, cx);
        }
    }

    #[doc(hidden)]
    pub fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.open = open;
        self.deferred_context = open.then(|| GlobalState::register_deferred_popover(cx));
    }

    #[doc(hidden)]
    pub fn sync_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.transition_to(open, false, window, cx);
    }

    #[doc(hidden)]
    pub fn toggle_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.transition_to(!self.open, true, window, cx);
    }

    fn transition_to(
        &mut self,
        opening: bool,
        announce: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open == opening {
            return;
        }
        self.hover.cancel();
        self.opened_by_hover = false;
        self.escape = None;
        self.set_open(opening, cx);

        if self.open {
            self.take_focus(window, cx);
            self.watch_dismiss(window, cx);
        } else {
            self.dismiss_subscription = None;
            self.focus_out_subscription = None;
            self.initial_focus = InitialFocus::Idle;
            let previous = self.previous_focus_handle.take();
            if self.focus_handle.contains_focused(window, cx) {
                match previous {
                    Some(previous) => previous.focus(window, cx),
                    // Nothing had focus when the popover opened, so nothing has it now. Left
                    // alone, focus would stay on the closed popover's own handle, which is no
                    // longer in the tree.
                    None => window.blur(cx),
                }
            }
        }

        if announce {
            self.announce(window, cx);
        }
        cx.notify();
    }

    /// Opens under a resting pointer. The focus stays where it is, as it does
    /// for Base UI's popover opened by hover, so Escape is heard from any
    /// focus in the window.
    fn open_by_hover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            return;
        }
        self.set_open(true, cx);
        self.opened_by_hover = true;
        let popover = cx.entity().downgrade();
        let home = window.window_handle();
        self.escape = Some(cx.intercept_keystrokes(move |event, window, cx| {
            let key = &event.keystroke;
            if key.key == "escape" && !key.modifiers.modified() && window.window_handle() == home {
                popover
                    .update(cx, |state, cx| state.dismiss(window, cx))
                    .ok();
            }
        }));
        self.watch_dismiss(window, cx);
        self.announce(window, cx);
        cx.notify();
    }

    /// Activating the trigger of a popover the pointer opened keeps it open
    /// as if activated to open: the pointer leaving no longer closes it, and
    /// it takes the focus. Base UI's `stickIfOpen`.
    fn stick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.hover.cancel();
        self.opened_by_hover = false;
        self.escape = None;
        self.take_focus(window, cx);
        cx.notify();
    }

    /// Activates the trigger of a popover rendered open or not.
    fn activate_trigger(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.opened_by_hover {
            self.stick(window, cx);
        } else {
            self.toggle_open(window, cx);
        }
    }

    fn take_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.previous_focus_handle = window.focused(cx);
        let focused = self
            .tracked_focus_handle
            .clone()
            .unwrap_or_else(|| self.focus_handle.clone());
        focused.focus(window, cx);
        // A caller's own focus target (a menu) keeps the focus; plain
        // content hands it to its first control.
        self.initial_focus = if self.tracked_focus_handle.is_none() {
            InitialFocus::Armed
        } else {
            InitialFocus::Idle
        };
        let content = self.focus_handle.clone();
        self.focus_out_subscription =
            Some(
                cx.on_focus_out(&focused, window, move |state, _, window, cx| {
                    // Within the popover (its content, a popup nested in it) is
                    // not out of it.
                    if !content.contains_focused(window, cx) {
                        state.dismiss(window, cx);
                        window.refresh();
                    }
                }),
            );
    }

    fn watch_dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Weak: the subscription is stored on this state, so a strong handle
        // would keep the state, and its deferred-popover registration, alive
        // after its trigger is gone.
        let state = cx.entity().downgrade();
        self.dismiss_subscription =
            Some(
                window.subscribe(&cx.entity(), cx, move |_, _: &DismissEvent, window, cx| {
                    _ = state.update(cx, |state, cx| state.dismiss(window, cx));
                    window.refresh();
                }),
            );
    }

    fn announce(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(callback) = self.on_open_change.clone() {
            callback(&self.open, window, cx);
        }
    }

    fn sync_hover(&mut self, open_on_hover: bool, delay: Duration, close_delay: Duration) {
        self.open_on_hover = open_on_hover;
        self.hover.open_delay = delay;
        self.hover.close_delay = close_delay;
    }

    #[doc(hidden)]
    pub fn track_focus(&mut self, focus_handle: Option<FocusHandle>) {
        self.tracked_focus_handle = focus_handle;
    }

    #[doc(hidden)]
    pub fn set_on_open_change(&mut self, handler: Option<OpenChangeHandler>) {
        self.on_open_change = handler;
    }

    #[doc(hidden)]
    pub fn on_action_cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss(window, cx);
    }
}

impl HoverHost for PopoverState {
    fn hover(&mut self) -> &mut HoverOpen {
        &mut self.hover
    }

    fn hover_wants_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open_on_hover {
            return;
        }
        if open {
            self.open_by_hover(window, cx);
        } else if self.opened_by_hover {
            self.transition_to(false, true, window, cx);
        }
    }
}

impl Focusable for PopoverState {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PopoverState {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl EventEmitter<DismissEvent> for PopoverState {}

type TriggerBuilder = Box<dyn FnOnce(bool, &Window, &App) -> AnyElement>;
type ContentBuilder =
    Box<dyn FnOnce(&mut PopoverState, &mut Window, &mut Context<PopoverState>) -> AnyElement>;

/// An unstyled popover owning trigger interaction, open state, dismissal, and focus.
///
/// Enter and Space on the trigger toggle the popover. Inside its content, those
/// keys belong to the focused control instead; Escape dismisses the popover and
/// restores the previous focus.
#[derive(IntoElement)]
pub struct Popover {
    id: ElementId,
    style: StyleRefinement,
    anchor: Anchor,
    offset: gpui::Pixels,
    on_position: Option<Box<dyn Fn(ResolvedPosition, gpui::Bounds<gpui::Pixels>)>>,
    default_open: bool,
    open: Option<bool>,
    tracked_focus_handle: Option<FocusHandle>,
    trigger: Option<TriggerBuilder>,
    content: Option<ContentBuilder>,
    mouse_button: MouseButton,
    overlay_closable: bool,
    on_open_change: Option<OpenChangeHandler>,
    open_on_hover: bool,
    hover_delay: Duration,
    hover_close_delay: Duration,
    motion: Transition,
    approach: gpui::Pixels,
}

impl Popover {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            style: StyleRefinement::default(),
            anchor: Anchor::TopLeft,
            offset: gpui::px(0.),
            on_position: None,
            default_open: false,
            open: None,
            tracked_focus_handle: None,
            trigger: None,
            content: None,
            mouse_button: MouseButton::Left,
            overlay_closable: true,
            on_open_change: None,
            open_on_hover: false,
            hover_delay: HOVER_DELAY,
            hover_close_delay: Duration::ZERO,
            motion: Transition::new(Duration::ZERO),
            approach: gpui::px(0.),
        }
    }

    /// The timing of the content's entry and exit. Closing keeps the content
    /// drawn until the exit ends, as Base UI keeps a closing popup mounted for
    /// its ending style; reopening on the way out turns the motion around.
    /// The content draws its look from [`PopoverState::motion_progress`].
    /// None by default: the content comes and goes in one frame.
    pub fn motion(mut self, motion: Transition) -> Self {
        self.motion = motion;
        self
    }

    /// How far back toward the trigger the content is drawn at the start of
    /// its [`Self::motion`], closing to nothing as it settles, so it comes
    /// out of the trigger and returns into it.
    pub fn approach(mut self, distance: gpui::Pixels) -> Self {
        self.approach = distance;
        self
    }

    /// Opens the popover while the pointer rests on its trigger (Base UI's
    /// `openOnHover`), leaving the focus where it is. The pointer may travel
    /// from the trigger to the content; leaving both closes it. Activating
    /// the trigger of a popover opened this way keeps it open and gives it the
    /// focus, as activating a closed one does.
    pub fn open_on_hover(mut self, open_on_hover: bool) -> Self {
        self.open_on_hover = open_on_hover;
        self
    }

    /// How long the pointer rests on the trigger before an
    /// [`Self::open_on_hover`] popover opens, 300 ms by default.
    pub fn hover_delay(mut self, delay: Duration) -> Self {
        self.hover_delay = delay;
        self
    }

    /// How long an [`Self::open_on_hover`] popover stays after the pointer
    /// left the trigger and the content, none by default.
    pub fn hover_close_delay(mut self, delay: Duration) -> Self {
        self.hover_close_delay = delay;
        self
    }

    pub fn anchor(mut self, anchor: impl Into<Anchor>) -> Self {
        self.anchor = anchor.into();
        self
    }

    /// Gap from the trigger along the anchor's outward direction, zero by default.
    pub fn offset(mut self, offset: gpui::Pixels) -> Self {
        self.offset = offset;
        self
    }

    /// Observe geometry to supply presentation such as a pointer arrow.
    pub fn on_position(
        mut self,
        callback: impl Fn(ResolvedPosition, gpui::Bounds<gpui::Pixels>) + 'static,
    ) -> Self {
        self.on_position = Some(Box::new(callback));
        self
    }

    pub fn mouse_button(mut self, mouse_button: MouseButton) -> Self {
        self.mouse_button = mouse_button;
        self
    }

    pub fn trigger<T>(mut self, trigger: T) -> Self
    where
        T: Selectable + IntoElement + 'static,
    {
        self.trigger = Some(Box::new(|is_open, _, _| {
            let open = trigger.is_open();
            trigger.open(open || is_open).into_any_element()
        }));
        self
    }

    /// Supplies a trigger builder for higher-level presentation facades.
    #[doc(hidden)]
    pub fn trigger_with(
        mut self,
        trigger: impl FnOnce(bool, &Window, &App) -> AnyElement + 'static,
    ) -> Self {
        self.trigger = Some(Box::new(trigger));
        self
    }

    pub fn default_open(mut self, open: bool) -> Self {
        self.default_open = open;
        self
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = Some(open);
        self
    }

    pub fn track_focus(mut self, handle: &FocusHandle) -> Self {
        self.tracked_focus_handle = Some(handle.clone());
        self
    }

    pub fn overlay_closable(mut self, closable: bool) -> Self {
        self.overlay_closable = closable;
        self
    }

    pub fn on_open_change(
        mut self,
        callback: impl Fn(&bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open_change = Some(Rc::new(callback));
        self
    }

    pub fn content<F, E>(mut self, content: F) -> Self
    where
        E: IntoElement,
        F: FnOnce(&mut PopoverState, &mut Window, &mut Context<PopoverState>) -> E + 'static,
    {
        self.content = Some(Box::new(move |state, window, cx| {
            content(state, window, cx).into_any_element()
        }));
        self
    }
}

/// Styles the trigger container: the element that takes part in the parent
/// layout and whose bounds the popup is anchored to.
impl Styled for Popover {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Popover {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| {
            PopoverState::new(self.default_open, cx)
        });
        state.update(cx, |state, cx| {
            state.track_focus(self.tracked_focus_handle);
            state.set_on_open_change(self.on_open_change);
            state.sync_hover(self.open_on_hover, self.hover_delay, self.hover_close_delay);
            if let Some(open) = self.open {
                state.sync_open(open, window, cx);
            }
        });

        let open = state.read(cx).is_open();
        let focus_handle = state.read(cx).focus_handle(cx);
        let presence = Presence::new(self.id.clone(), open)
            .transition(self.motion)
            .sample(window, cx);
        let exiting = presence.phase == PresencePhase::Exiting;
        state.update(cx, |state, _| {
            state.motion_progress = presence.progress;
            state.exiting = exiting;
        });
        let Some(trigger) = self.trigger else {
            return div().id("empty").into_any_element();
        };
        let parent_view_id = window.current_view();
        let popup = Popup::new(self.id, trigger(open, window, cx))
            .refine_style(&self.style)
            .anchor(self.anchor)
            .offset(self.offset)
            .approach(self.approach * (1. - presence.progress))
            .when_some(self.on_position, |this, callback| {
                this.on_position(callback)
            })
            .key_context(CONTEXT)
            .on_action({
                let state = state.clone();
                move |_: &Confirm, window, cx| {
                    state.update(cx, |state, cx| state.activate_trigger(window, cx));
                    cx.notify(parent_view_id);
                }
            })
            .on_mouse_down(self.mouse_button, {
                let state = state.clone();
                move |_, window, cx| {
                    cx.stop_propagation();
                    state.update(cx, |state, cx| {
                        if state.is_open() == open {
                            state.activate_trigger(window, cx);
                        }
                    });
                    cx.notify(parent_view_id);
                }
            })
            .when(self.open_on_hover, |popup| {
                popup.on_hover(window.listener_for(&state, |state, hovered, window, cx| {
                    let open = state.open;
                    hover_open::trigger_hover(state, *hovered, open, window, cx);
                }))
            });
        if !presence.should_render() {
            return popup.into_any_element();
        }

        let content = div()
            .id("content")
            // A popover surface is a non-modal dialog: it takes focus and is
            // dismissed with Escape, which is what this role tells assistive
            // technology to expect.
            .role(Role::Dialog)
            .occlude()
            .tab_group()
            .track_focus(&focus_handle)
            .key_context(CONTENT_CONTEXT)
            .on_action(window.listener_for(&state, PopoverState::on_action_cancel))
            .when_some(self.content, |this, content| {
                this.child(state.update(cx, |state, cx| (content)(state, window, cx)))
            })
            .when(self.open_on_hover && !exiting, |this| {
                this.relative()
                    .on_hover(window.listener_for(&state, |state, hovered, window, cx| {
                        hover_open::popup_hover(state, *hovered, window, cx);
                    }))
                    .child(hover_open::travel_watcher(state.downgrade()))
            })
            .when(self.overlay_closable && !exiting, |this| {
                this.on_mouse_down_out({
                    let state = state.clone();
                    move |_, window, cx| {
                        state.update(cx, |state, cx| {
                            // A press on the trigger of a popover the pointer
                            // opened keeps it: the trigger's own handler.
                            if !(state.opened_by_hover && state.hover.over_trigger()) {
                                state.dismiss(window, cx);
                            }
                        });
                        cx.notify(parent_view_id);
                    }
                })
            });
        // After the content was built: content that placed the focus itself
        // (a menu) keeps it.
        state.update(cx, |state, cx| {
            let mut initial_focus = state.initial_focus;
            initial_focus.render(&focus_handle, window, cx);
            state.initial_focus = initial_focus;
        });
        // Closing, it takes no pointer (Base UI's `pointer-events: none`):
        // a press during the fade reaches what lies beneath.
        popup.inert(exiting).content(content).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, Context, Render, point, px};
    use std::{cell::RefCell, rc::Rc};

    /// Popover state lives in element state, which is collected as soon as it
    /// stops being rendered — a trigger scrolled out of a virtual list, a panel
    /// closed with its menu open. A registration that outlived it would leave
    /// the application believing a popup is open for the rest of the session.
    #[gpui::test]
    fn a_state_dropped_while_open_closes_the_deferred_context(cx: &mut gpui::TestAppContext) {
        let state = cx.update(|cx| {
            GlobalState::init(cx);
            let state = cx.new(|cx| PopoverState::new(false, cx));
            state.update(cx, |state, cx| state.set_open(true, cx));
            assert!(GlobalState::is_in_deferred_context(cx));
            state
        });

        cx.update(|_| drop(state));
        cx.update(|cx| assert!(!GlobalState::is_in_deferred_context(cx)));
    }

    #[gpui::test]
    fn open_state_registers_and_unregisters_deferred_context(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            GlobalState::init(cx);
            let state = cx.new(|cx| PopoverState::new(false, cx));

            state.update(cx, |state, cx| state.set_open(true, cx));
            assert!(state.read(cx).is_open());
            assert!(GlobalState::is_in_deferred_context(cx));

            state.update(cx, |state, cx| state.set_open(false, cx));
            assert!(!state.read(cx).is_open());
            assert!(!GlobalState::is_in_deferred_context(cx));
        });
    }

    struct PopoverHarness {
        changes: Rc<RefCell<Vec<bool>>>,
        default_open: bool,
    }

    struct KeyboardPopoverHarness {
        trigger_focus: FocusHandle,
    }

    impl Render for KeyboardPopoverHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            Popover::new("keyboard-popover")
                .trigger(
                    crate::Button::new("keyboard-trigger")
                        .track_focus(&self.trigger_focus)
                        .child("Open"),
                )
                .content(|_, _, _| {
                    div()
                        .debug_selector(|| "keyboard-popover-content".into())
                        .size(px(40.))
                })
        }
    }

    impl Render for PopoverHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let changes = self.changes.clone();
            Popover::new("base-popover")
                .default_open(self.default_open)
                .trigger_with(|_, _, _| div().child("Open").into_any_element())
                .content(|_, _, _| {
                    div()
                        .debug_selector(|| "base-popover-content".into())
                        .size(px(40.))
                })
                .on_open_change(move |open, _, _| changes.borrow_mut().push(*open))
        }
    }

    #[gpui::test]
    fn unstyled_popover_owns_pointer_open_and_outside_dismiss(cx: &mut gpui::TestAppContext) {
        cx.update(crate::init);
        let changes = Rc::new(RefCell::new(Vec::new()));
        let (_, cx) = cx.add_window_view({
            let changes = changes.clone();
            move |_, _| PopoverHarness {
                changes,
                default_open: false,
            }
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| assert!(window.focused(cx).is_none(), "nothing has focus yet"));

        cx.simulate_click(point(px(20.), px(10.)), Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("base-popover-content").is_some());
        cx.update(|window, cx| assert!(window.focused(cx).is_some(), "the popover takes focus"));

        cx.simulate_click(point(px(300.), px(300.)), Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("base-popover-content").is_none());
        assert_eq!(&*changes.borrow(), &[true, false]);
        // Nothing had focus when the popover opened, so nothing has it now, not the closed
        // popover's own handle.
        cx.update(|window, cx| {
            assert!(
                window.focused(cx).is_none(),
                "focus stays on {:?}",
                window.focused(cx)
            )
        });
    }

    #[gpui::test]
    fn default_open_renders_content_without_activation(cx: &mut gpui::TestAppContext) {
        cx.update(crate::init);
        let (_, cx) = cx.add_window_view(|_, _| PopoverHarness {
            changes: Rc::new(RefCell::new(Vec::new())),
            default_open: true,
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("base-popover-content").is_some());
    }

    /// A trigger that keeps "open" and "selected" apart, the way a downstream
    /// sidebar row does: it is selected when it is the current view, and open
    /// only while its popover is showing.
    #[derive(IntoElement)]
    struct RecordingTrigger {
        calls: Rc<RefCell<Vec<(&'static str, bool)>>>,
        selected: bool,
        open: bool,
    }

    impl Selectable for RecordingTrigger {
        fn selected(mut self, selected: bool) -> Self {
            self.calls.borrow_mut().push(("selected", selected));
            self.selected = selected;
            self
        }

        fn is_selected(&self) -> bool {
            self.selected
        }

        fn open(mut self, open: bool) -> Self {
            self.calls.borrow_mut().push(("open", open));
            self.open = open;
            self
        }

        fn is_open(&self) -> bool {
            self.open
        }
    }

    impl RenderOnce for RecordingTrigger {
        fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
            div().size(px(40.)).child("Open")
        }
    }

    struct RecordingTriggerHarness {
        calls: Rc<RefCell<Vec<(&'static str, bool)>>>,
    }

    impl Render for RecordingTriggerHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            Popover::new("recording-popover")
                .trigger(RecordingTrigger {
                    calls: self.calls.clone(),
                    selected: false,
                    open: false,
                })
                .content(|_, _, _| div().size(px(40.)))
        }
    }

    #[gpui::test]
    fn an_open_popover_tells_its_trigger_it_is_open_not_selected(cx: &mut gpui::TestAppContext) {
        cx.update(crate::init);
        let calls = Rc::new(RefCell::new(Vec::new()));
        let (_, cx) = cx.add_window_view({
            let calls = calls.clone();
            move |_, _| RecordingTriggerHarness { calls }
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        calls.borrow_mut().clear();

        cx.simulate_click(point(px(20.), px(10.)), Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));

        let calls = calls.borrow();
        assert!(
            calls.contains(&("open", true)),
            "an open popover marks its trigger open, got {calls:?}"
        );
        assert!(
            !calls.iter().any(|(name, _)| *name == "selected"),
            "opening must not touch the trigger's own selection, got {calls:?}"
        );
    }

    #[gpui::test]
    fn keyboard_activation_opens_the_popover(cx: &mut gpui::TestAppContext) {
        cx.update(crate::init);
        let (view, cx) = cx.add_window_view(|_, cx| KeyboardPopoverHarness {
            trigger_focus: cx.focus_handle(),
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));

        cx.update(|window, cx| {
            let focus = view.read(cx).trigger_focus.clone();
            focus.focus(window, cx);
        });
        cx.simulate_keystrokes("enter");
        cx.update(|window, cx| window.draw(cx).clear(cx));

        assert!(cx.debug_bounds("keyboard-popover-content").is_some());
    }

    /// A popover that opens on hover, below a 40 x 20 trigger at the window's
    /// top-left, 10 pt away: its content spans y 30 to 70.
    struct HoverHarness {
        changes: Rc<RefCell<Vec<bool>>>,
        elsewhere: FocusHandle,
    }

    impl Render for HoverHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let changes = self.changes.clone();
            // A row that does not stretch the trigger across the window.
            div()
                .size_full()
                .flex()
                .items_start()
                .child(
                    Popover::new("hover-popover")
                        .open_on_hover(true)
                        .offset(px(10.))
                        .trigger_with(|_, _, _| div().size(px(40.)).h(px(20.)).into_any_element())
                        .content(|_, _, _| {
                            div()
                                .debug_selector(|| "hover-popover-content".into())
                                .size(px(40.))
                        })
                        .on_open_change(move |open, _, _| changes.borrow_mut().push(*open)),
                )
                .child(
                    div()
                        .id("elsewhere")
                        .track_focus(&self.elsewhere)
                        .size(px(10.)),
                )
        }
    }

    fn hover_harness(
        cx: &mut gpui::TestAppContext,
    ) -> (
        Rc<RefCell<Vec<bool>>>,
        FocusHandle,
        &mut gpui::VisualTestContext,
    ) {
        cx.update(crate::init);
        let changes = Rc::new(RefCell::new(Vec::new()));
        let (view, cx) = cx.add_window_view({
            let changes = changes.clone();
            move |window, cx| {
                let elsewhere = cx.focus_handle();
                elsewhere.focus(window, cx);
                HoverHarness { changes, elsewhere }
            }
        });
        let elsewhere = view.read_with(cx, |view, _| view.elsewhere.clone());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        (changes, elsewhere, cx)
    }

    fn settle(cx: &mut gpui::VisualTestContext, by: Duration) {
        cx.executor().advance_clock(by);
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn shown(cx: &mut gpui::VisualTestContext) -> bool {
        cx.debug_bounds("hover-popover-content").is_some()
    }

    #[gpui::test]
    fn resting_on_the_trigger_opens_it_and_leaves_the_focus(cx: &mut gpui::TestAppContext) {
        let (changes, elsewhere, cx) = hover_harness(cx);
        cx.simulate_mouse_move(point(px(20.), px(10.)), None, Default::default());
        settle(cx, HOVER_DELAY - Duration::from_millis(1));
        assert!(!shown(cx), "not before the delay");
        settle(cx, Duration::from_millis(1));
        assert!(shown(cx));
        assert_eq!(&*changes.borrow(), &[true]);
        cx.update(|window, _| assert!(elsewhere.is_focused(window), "the focus stays put"));

        // Off to the side, away from the content: it closes at once.
        cx.simulate_mouse_move(point(px(300.), px(5.)), None, Default::default());
        settle(cx, Duration::from_millis(100));
        assert!(!shown(cx));
        assert_eq!(&*changes.borrow(), &[true, false]);
    }

    #[gpui::test]
    fn a_trip_to_the_content_keeps_it_and_resting_on_the_way_closes_it(
        cx: &mut gpui::TestAppContext,
    ) {
        let (_, _, cx) = hover_harness(cx);
        cx.simulate_mouse_move(point(px(20.), px(10.)), None, Default::default());
        settle(cx, HOVER_DELAY);
        assert!(shown(cx));

        // Down across the gap, a step at a time with a pause shorter than a
        // rest between them, into the content.
        for y in [22., 25., 28.] {
            cx.simulate_mouse_move(point(px(20.), px(y)), None, Default::default());
            settle(cx, Duration::from_millis(20));
            assert!(shown(cx), "still on its way at y {y}");
        }
        cx.simulate_mouse_move(point(px(20.), px(40.)), None, Default::default());
        settle(cx, Duration::from_millis(500));
        assert!(shown(cx), "over the content it stays");

        // Back up to the trigger, then into the gap again and rest there.
        cx.simulate_mouse_move(point(px(20.), px(10.)), None, Default::default());
        settle(cx, Duration::from_millis(20));
        cx.simulate_mouse_move(point(px(20.), px(24.)), None, Default::default());
        settle(cx, Duration::from_millis(100));
        assert!(!shown(cx), "resting on the way closes it");
    }

    #[gpui::test]
    fn pressing_the_trigger_keeps_a_hover_opened_popover(cx: &mut gpui::TestAppContext) {
        let (changes, elsewhere, cx) = hover_harness(cx);
        cx.simulate_mouse_move(point(px(20.), px(10.)), None, Default::default());
        settle(cx, HOVER_DELAY);
        cx.simulate_click(point(px(20.), px(10.)), Default::default());
        settle(cx, Duration::ZERO);
        assert!(shown(cx));
        cx.update(|window, _| assert!(!elsewhere.is_focused(window), "it takes the focus"));

        cx.simulate_mouse_move(point(px(300.), px(5.)), None, Default::default());
        settle(cx, Duration::from_millis(500));
        assert!(shown(cx), "the pointer leaving no longer closes it");
        assert_eq!(&*changes.borrow(), &[true]);
    }

    #[gpui::test]
    fn escape_closes_a_hover_opened_popover_from_any_focus(cx: &mut gpui::TestAppContext) {
        let (changes, _, cx) = hover_harness(cx);
        cx.simulate_mouse_move(point(px(20.), px(10.)), None, Default::default());
        settle(cx, HOVER_DELAY);
        assert!(shown(cx));
        cx.simulate_keystrokes("escape");
        settle(cx, Duration::ZERO);
        assert!(!shown(cx));
        assert_eq!(&*changes.borrow(), &[true, false]);

        // Still over the trigger: it waits for the pointer to come back.
        settle(cx, HOVER_DELAY * 2);
        assert!(!shown(cx));
    }

    /// A popover with a 100 ms motion whose content reports its progress.
    struct MotionHarness {
        progress: Rc<std::cell::Cell<Option<(f32, bool)>>>,
        beneath: Rc<std::cell::Cell<usize>>,
    }

    impl Render for MotionHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let progress = self.progress.clone();
            let beneath = self.beneath.clone();
            div()
                .size_full()
                .child(
                    div()
                        .id("beneath")
                        .absolute()
                        .size_full()
                        .on_click(move |_, _, _| beneath.set(beneath.get() + 1)),
                )
                .child(self.popover(progress))
        }
    }

    impl MotionHarness {
        fn popover(&self, progress: Rc<std::cell::Cell<Option<(f32, bool)>>>) -> Popover {
            Popover::new("motion-popover")
                .motion(Transition::new(Duration::from_millis(100)))
                .trigger_with(|_, _, _| div().size(px(40.)).into_any_element())
                .content(move |state, _, _| {
                    progress.set(Some((state.motion_progress(), state.is_exiting())));
                    div()
                        .debug_selector(|| "motion-popover-content".into())
                        .size(px(40.))
                })
        }
    }

    fn motion_harness(
        cx: &mut gpui::TestAppContext,
    ) -> (
        Rc<std::cell::Cell<Option<(f32, bool)>>>,
        Rc<std::cell::Cell<usize>>,
        &mut gpui::VisualTestContext,
    ) {
        cx.update(crate::init);
        let progress = Rc::new(std::cell::Cell::new(None));
        let beneath = Rc::new(std::cell::Cell::new(0));
        let (_, cx) = cx.add_window_view({
            let progress = progress.clone();
            let beneath = beneath.clone();
            move |_, _| MotionHarness { progress, beneath }
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        (progress, beneath, cx)
    }

    /// Closing keeps the content drawn, exiting, until its motion ends;
    /// reopening on the way out turns it around without a frame of absence.
    #[gpui::test]
    fn a_closing_popover_stays_drawn_through_its_exit(cx: &mut gpui::TestAppContext) {
        let (progress, _, cx) = motion_harness(cx);
        assert_eq!(progress.get(), None, "a closed popover draws no content");

        cx.simulate_click(point(px(20.), px(20.)), Default::default());
        settle(cx, Duration::from_millis(100));
        assert_eq!(progress.get(), Some((1., false)));

        cx.simulate_click(point(px(300.), px(300.)), Default::default());
        settle(cx, Duration::from_millis(50));
        let (half, exiting) = progress.get().unwrap();
        assert!(exiting && half > 0. && half < 1., "exiting at {half}");
        assert!(cx.debug_bounds("motion-popover-content").is_some());

        cx.simulate_click(point(px(20.), px(20.)), Default::default());
        settle(cx, Duration::ZERO);
        let (back, exiting) = progress.get().unwrap();
        assert!(!exiting && back > 0., "reopened from {back}");

        cx.simulate_click(point(px(300.), px(300.)), Default::default());
        settle(cx, Duration::from_millis(200));
        assert!(cx.debug_bounds("motion-popover-content").is_none());
    }

    /// An open popover takes a press on its content; closing, it lets the
    /// press through to what lies beneath, as Base UI's positioner does with
    /// `pointer-events: none`, and goes on closing.
    #[gpui::test]
    fn a_closing_popover_lets_presses_through(cx: &mut gpui::TestAppContext) {
        let (progress, beneath, cx) = motion_harness(cx);
        cx.simulate_click(point(px(20.), px(20.)), Default::default());
        settle(cx, Duration::from_millis(100));
        let content = cx.debug_bounds("motion-popover-content").unwrap().center();
        cx.simulate_click(content, Default::default());
        settle(cx, Duration::ZERO);
        assert_eq!(beneath.get(), 0, "the open content takes the press");
        assert_eq!(progress.get(), Some((1., false)));

        cx.simulate_click(point(px(300.), px(300.)), Default::default());
        settle(cx, Duration::from_millis(30));
        assert_eq!(beneath.get(), 1);
        cx.simulate_click(content, Default::default());
        settle(cx, Duration::from_millis(30));
        assert_eq!(beneath.get(), 2, "the closing content lets it through");
        assert!(progress.get().unwrap().1, "and goes on closing");
    }
}
