use std::rc::Rc;

use gpui::{
    AccessibleAction, AnyElement, App, ElementId, FocusHandle, InteractiveElement as _,
    IntoElement, KeyBinding, ParentElement, RenderOnce, Role, SharedString,
    StatefulInteractiveElement as _, StyleRefinement, Styled, Window, div,
    prelude::FluentBuilder as _,
};

use crate::{
    StyledExt as _, TestSupportExt as _,
    a11y_disabled::A11yDisabledExt as _,
    actions::{Cancel, Confirm, SelectDown, SelectUp},
};

const CONTEXT: &str = "Combobox";

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", SelectUp, Some(CONTEXT)),
        KeyBinding::new("down", SelectDown, Some(CONTEXT)),
        KeyBinding::new("enter", Confirm { secondary: false }, Some(CONTEXT)),
        KeyBinding::new(
            if cx.platform_info().is_mac() {
                "cmd-enter"
            } else {
                "ctrl-enter"
            },
            Confirm { secondary: true },
            Some(CONTEXT),
        ),
        KeyBinding::new("escape", Cancel, Some(CONTEXT)),
    ]);
}

/// Where an arrow key moves a combobox's highlight.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HighlightStep {
    /// Down in an open list: the next item.
    Next,
    /// Up in an open list: the previous item.
    Previous,
    /// Down opened the list: the selected item, or else the first.
    SelectedOrFirst,
    /// Up opened the list: the selected item, or else the last.
    SelectedOrLast,
}

type OpenChangeHandler = Rc<dyn Fn(bool, &mut Window, &mut App)>;
type HighlightHandler = Rc<dyn Fn(HighlightStep, &mut Window, &mut App)>;
type ActionHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// An unstyled controlled combobox root, with its text input in the field as
/// Base UI's `Combobox.Input` is.
///
/// The application puts the input inside this root and owns the list, the
/// filtering, the selection and the appearance. Focus stays in the input the
/// whole time: this root takes no focus of its own, and it handles the keys
/// the input leaves alone, so the caret never leaves the field while arrows
/// move the highlight through the list.
///
/// - Down and Up open a closed list, then move the highlight
///   ([`Self::on_highlight`]).
/// - Enter in an open list commits the highlighted item ([`Self::on_confirm`]);
///   in a closed one it is left to whatever holds the combobox, a form say.
/// - Escape closes an open list ([`Self::on_dismiss`]), and in a closed one
///   clears the field ([`Self::on_clear`]).
///
/// Typing opens the list too, but only the input sees typing, so the
/// application opens it from the input's change.
#[derive(IntoElement)]
pub struct Combobox {
    base: crate::ObservedElement<gpui::Stateful<gpui::Div>>,
    open: bool,
    disabled: bool,
    focus_handle: Option<FocusHandle>,
    accessibility_label: Option<SharedString>,
    accessibility_value: Option<SharedString>,
    style: StyleRefinement,
    children: Vec<AnyElement>,
    on_open_change: Option<OpenChangeHandler>,
    on_highlight: Option<HighlightHandler>,
    on_confirm: Option<ActionHandler>,
    on_dismiss: Option<ActionHandler>,
    on_clear: Option<ActionHandler>,
}

impl Combobox {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(id).test_support(),
            open: false,
            disabled: false,
            focus_handle: None,
            accessibility_label: None,
            accessibility_value: None,
            style: StyleRefinement::default(),
            children: Vec::new(),
            on_open_change: None,
            on_highlight: None,
            on_confirm: None,
            on_dismiss: None,
            on_clear: None,
        }
    }

    /// Sets the application-controlled open state.
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// Ignores the keys and the accessible activation.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Supplies a handle the root tracks as the field's focus scope.
    ///
    /// Focus lives in the input inside the field, never on this handle, and
    /// the root is no tab stop. Tracking it lets the application ask whether
    /// focus is anywhere in the field, its input or what sits beside it.
    pub fn focus_handle(mut self, focus_handle: &FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle.clone());
        self
    }

    /// Sets the accessible name exposed by the root.
    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessibility_label = Some(label.into());
        self
    }

    /// Sets the committed value exposed by the root: a readable label of the
    /// selection, not what is typed in the field.
    pub fn accessibility_value(mut self, value: impl Into<SharedString>) -> Self {
        self.accessibility_value = Some(value.into());
        self
    }

    /// Handles requests to update the controlled open state.
    pub fn on_open_change(
        mut self,
        handler: impl Fn(bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open_change = Some(Rc::new(handler));
        self
    }

    /// Handles Down and Up, which move the highlight through the list.
    ///
    /// In a closed list the key asks to open it first and then runs this
    /// with [`HighlightStep::SelectedOrFirst`] or
    /// [`HighlightStep::SelectedOrLast`].
    pub fn on_highlight(
        mut self,
        handler: impl Fn(HighlightStep, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_highlight = Some(Rc::new(handler));
        self
    }

    /// Handles the Confirm action while the list is open: commit the
    /// highlighted item.
    pub fn on_confirm(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_confirm = Some(Rc::new(handler));
        self
    }

    /// Handles a dismissal of the open list, through Escape or the accessible
    /// activation.
    ///
    /// This runs before the controlled open state is asked to close, so a
    /// combobox that commits its pending value on dismissal can still read
    /// that value here.
    pub fn on_dismiss(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(handler));
        self
    }

    /// Handles Escape while the list is closed: clear the field.
    ///
    /// Supply it only while there is something to clear. Without it Escape
    /// in a closed combobox goes on to whatever holds it, a dialog say.
    pub fn on_clear(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_clear = Some(Rc::new(handler));
        self
    }
}

impl Styled for Combobox {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Combobox {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Combobox {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let open = self.open;
        let disabled = self.disabled;
        let on_open_change = self.on_open_change;
        let on_highlight = self.on_highlight;
        let on_confirm = self.on_confirm;
        let on_dismiss = self.on_dismiss;
        let on_clear = self.on_clear;

        let request_open_change = move |open: bool, window: &mut Window, cx: &mut App| {
            if let Some(handler) = on_open_change.as_ref() {
                handler(open, window, cx);
            }
        };

        // Every way of closing runs the same steps, so a caller that commits
        // on dismissal sees one however the list was closed.
        let close: ActionHandler = Rc::new({
            let request_open_change = request_open_change.clone();
            move |window: &mut Window, cx: &mut App| {
                if let Some(handler) = on_dismiss.as_ref() {
                    handler(window, cx);
                }
                request_open_change(false, window, cx);
            }
        });

        // Down and Up open a closed list on the selected item, or move the
        // highlight through an open one.
        let step: HighlightHandler = Rc::new({
            let request_open_change = request_open_change.clone();
            move |step: HighlightStep, window: &mut Window, cx: &mut App| {
                let step = match (open, step) {
                    (true, step) => step,
                    (false, HighlightStep::Previous | HighlightStep::SelectedOrLast) => {
                        request_open_change(true, window, cx);
                        HighlightStep::SelectedOrLast
                    }
                    (false, HighlightStep::Next | HighlightStep::SelectedOrFirst) => {
                        request_open_change(true, window, cx);
                        HighlightStep::SelectedOrFirst
                    }
                };
                if let Some(handler) = on_highlight.as_ref() {
                    handler(step, window, cx);
                }
            }
        });

        self.base
            .role(Role::ComboBox)
            .aria_expanded(open)
            .when_some(self.accessibility_label, |this, label| {
                this.aria_label(label)
            })
            .when_some(self.accessibility_value, |this, value| {
                this.aria_value(value)
            })
            .key_context(CONTEXT)
            .when_some(self.focus_handle, |this, handle| {
                this.track_focus(&handle.tab_stop(false))
            })
            .when(!disabled, |this| {
                let close = close.clone();
                let request_open_change = request_open_change.clone();

                // Platform adapters may flatten the field. Expose activation
                // on the semantic root itself.
                this.on_a11y_action(AccessibleAction::Click, move |_, window, cx| {
                    if open {
                        close(window, cx);
                    } else {
                        request_open_change(true, window, cx);
                    }
                })
            })
            .on_action({
                let step = step.clone();
                move |_: &SelectDown, window, cx| {
                    if disabled {
                        cx.propagate();
                        return;
                    }
                    cx.stop_propagation();
                    step(HighlightStep::Next, window, cx);
                }
            })
            .on_action(move |_: &SelectUp, window, cx| {
                if disabled {
                    cx.propagate();
                    return;
                }
                cx.stop_propagation();
                step(HighlightStep::Previous, window, cx);
            })
            .on_action(move |_: &Confirm, window, cx| {
                if disabled || !open {
                    cx.propagate();
                    return;
                }
                cx.stop_propagation();
                if let Some(handler) = on_confirm.as_ref() {
                    handler(window, cx);
                }
            })
            .on_action(move |_: &Cancel, window, cx| {
                if disabled {
                    cx.propagate();
                    return;
                }
                if open {
                    cx.stop_propagation();
                    close(window, cx);
                } else if let Some(handler) = on_clear.as_ref() {
                    cx.stop_propagation();
                    handler(window, cx);
                } else {
                    cx.propagate();
                }
            })
            .children(self.children)
            .refine_style(&self.style)
            .a11y_disabled(disabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::InputState;
    use gpui::{
        AppContext as _, Context, Entity, Focusable, Render, TestAppContext, VisualTestContext,
    };
    use std::sync::{Arc, Mutex};

    /// A combobox whose field holds a text input, recording what the root
    /// asked of it.
    struct Harness {
        open: bool,
        clearable: bool,
        field_focus: FocusHandle,
        input: Entity<InputState>,
        events: Arc<Mutex<Vec<String>>>,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let state = cx.entity();
            let open_events = self.events.clone();
            let highlight_events = self.events.clone();
            let confirm_events = self.events.clone();
            let dismiss_events = self.events.clone();
            let clear_events = self.events.clone();

            Combobox::new("combobox")
                .open(self.open)
                .focus_handle(&self.field_focus)
                .on_open_change(move |open, _, cx| {
                    open_events
                        .lock()
                        .unwrap()
                        .push(if open { "open" } else { "close" }.into());
                    state.update(cx, |state, cx| {
                        state.open = open;
                        cx.notify();
                    });
                })
                .on_highlight(move |step, _, _| {
                    highlight_events.lock().unwrap().push(format!("{step:?}"));
                })
                .on_confirm(move |_, _| confirm_events.lock().unwrap().push("confirm".into()))
                .on_dismiss(move |_, _| dismiss_events.lock().unwrap().push("dismiss".into()))
                .when(self.clearable, |this| {
                    this.on_clear(move |_, _| clear_events.lock().unwrap().push("clear".into()))
                })
                .child(crate::Input::new(&self.input))
        }
    }

    fn harness(
        cx: &mut TestAppContext,
        clearable: bool,
    ) -> (&mut VisualTestContext, Entity<Harness>) {
        cx.update(crate::init);
        let (state, cx) = cx.add_window_view(move |window, cx| Harness {
            open: false,
            clearable,
            field_focus: cx.focus_handle(),
            input: cx.new(|cx| InputState::new(window, cx)),
            events: Arc::new(Mutex::new(Vec::new())),
        });
        cx.update(|window, cx| {
            let input = state.read(cx).input.clone();
            input.focus_handle(cx).focus(window, cx);
            window.draw(cx).clear(cx);
        });
        (cx, state)
    }

    fn events(state: &Entity<Harness>, cx: &mut VisualTestContext) -> Vec<String> {
        state.read_with(cx, |state, _| state.events.lock().unwrap().clone())
    }

    /// Down in a closed field opens the list on the selected item, then moves
    /// through it, while the caret stays in the input.
    #[gpui::test]
    fn arrows_open_then_move_the_highlight_from_the_input(cx: &mut TestAppContext) {
        let (cx, state) = harness(cx, false);
        cx.simulate_keystrokes("down");
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_keystrokes("down up");

        assert_eq!(
            events(&state, cx),
            ["open", "SelectedOrFirst", "Next", "Previous"]
        );
        cx.update(|window, cx| {
            let state = state.read(cx);
            assert!(state.open);
            assert!(state.input.focus_handle(cx).is_focused(window));
            assert!(state.field_focus.contains_focused(window, cx));
        });
    }

    #[gpui::test]
    fn up_opens_a_closed_list_from_the_end(cx: &mut TestAppContext) {
        let (cx, state) = harness(cx, false);
        cx.simulate_keystrokes("up");
        assert_eq!(events(&state, cx), ["open", "SelectedOrLast"]);
    }

    /// Typing stays text: the root takes only the keys the input leaves.
    #[gpui::test]
    fn typing_reaches_the_input(cx: &mut TestAppContext) {
        let (cx, state) = harness(cx, false);
        cx.simulate_keystrokes("r u space s");
        let input = state.read_with(cx, |state, _| state.input.clone());
        assert_eq!(input.read_with(cx, |input, _| input.value()), "ru s");
        assert!(events(&state, cx).is_empty());
    }

    #[gpui::test]
    fn enter_commits_only_while_open(cx: &mut TestAppContext) {
        let (cx, state) = harness(cx, false);
        cx.simulate_keystrokes("enter");
        assert!(
            events(&state, cx).is_empty(),
            "a closed field leaves Enter alone"
        );

        cx.simulate_keystrokes("down");
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_keystrokes("enter");
        assert_eq!(events(&state, cx), ["open", "SelectedOrFirst", "confirm"]);
    }

    /// Escape closes an open list, dismissing before it closes, and in a
    /// closed field clears it when there is something to clear.
    #[gpui::test]
    fn escape_closes_then_clears(cx: &mut TestAppContext) {
        let (cx, state) = harness(cx, true);
        cx.simulate_keystrokes("down");
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_keystrokes("escape");
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_keystrokes("escape");

        assert_eq!(
            events(&state, cx),
            ["open", "SelectedOrFirst", "dismiss", "close", "clear"]
        );
        assert!(!state.read_with(cx, |state, _| state.open));
    }

    #[gpui::test]
    fn escape_in_a_closed_field_with_nothing_to_clear_is_not_taken(cx: &mut TestAppContext) {
        let (cx, state) = harness(cx, false);
        cx.simulate_keystrokes("escape");
        assert!(events(&state, cx).is_empty());
    }
}
