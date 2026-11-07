use crate::input::{InputState, MoveEnd, MoveHome};
use std::{rc::Rc, time::Duration};

use gpui::Focusable;
use gpui::{
    AnyElement, App, ClickEvent, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    KeyBinding, MouseButton, MouseDownEvent, MouseUpEvent, ParentElement, RenderOnce, Role,
    SharedString, StatefulInteractiveElement as _, StyleRefinement, Styled, Task, Window, actions,
    div, prelude::FluentBuilder as _,
};

use crate::{Button, InputBase, StyledExt as _};

actions!(
    number_input,
    [
        Increment,
        Decrement,
        IncrementLarge,
        DecrementLarge,
        IncrementSmall,
        DecrementSmall
    ]
);

const CONTEXT: &str = "NumberInput";

/// How long a step button is held before it starts repeating.
const REPEAT_DELAY: Duration = Duration::from_millis(400);
/// How often a held step button steps once it repeats.
const REPEAT_INTERVAL: Duration = Duration::from_millis(60);

pub(crate) fn init(cx: &mut App) {
    // A single-line editor leaves these keys unhandled, so they reach the
    // number input around it; Base UI's Number Field takes the same ones.
    cx.bind_keys([
        KeyBinding::new("up", Increment, Some(CONTEXT)),
        KeyBinding::new("down", Decrement, Some(CONTEXT)),
        KeyBinding::new("shift-up", IncrementLarge, Some(CONTEXT)),
        KeyBinding::new("shift-down", DecrementLarge, Some(CONTEXT)),
        KeyBinding::new("alt-up", IncrementSmall, Some(CONTEXT)),
        KeyBinding::new("alt-down", DecrementSmall, Some(CONTEXT)),
    ]);
}

/// How far one step moves a number input.
#[derive(Clone, Copy, Debug, PartialEq)]
enum StepAmount {
    /// The input state's own step.
    Step,
    /// A fixed amount: the large step (Shift) or the small step (Option).
    By(f64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepAction {
    Decrement,
    Increment,
}

/// Strategy used by numeric editors when stepping their value.
#[derive(Clone)]
pub enum NumberStep {
    Fixed(f64),
    ByValue(Rc<dyn Fn(f64, StepAction, &mut gpui::App) -> f64>),
}

impl NumberStep {
    pub fn by_value(f: impl Fn(f64, StepAction, &mut gpui::App) -> f64 + 'static) -> Self {
        Self::ByValue(Rc::new(f))
    }

    pub(crate) fn value(&self, current: f64, action: StepAction, cx: &mut gpui::App) -> f64 {
        match self {
            Self::Fixed(step) => *step,
            Self::ByValue(f) => f(current, action, cx),
        }
    }
}

impl From<f64> for NumberStep {
    fn from(step: f64) -> Self {
        Self::Fixed(step)
    }
}

#[derive(Clone)]
pub enum NumberInputEvent {
    Step(StepAction),
}
impl EventEmitter<NumberInputEvent> for InputState {}

impl InputState {
    /// Apply a number-input step or emit a step event when stepping is caller-controlled.
    fn apply_number_step(
        &mut self,
        action: StepAction,
        amount: StepAmount,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        if let Some(step) = self.number_step.clone() {
            let value = self.unmask_value();
            let current = value.trim().parse::<f64>().unwrap_or(0.);
            let step = match amount {
                StepAmount::Step => step.value(current, action, cx),
                StepAmount::By(amount) => amount,
            };
            if let Some(new_value) =
                step_value(&value, action, step, self.number_min, self.number_max)
            {
                if self.is_valid_input(&new_value, cx) {
                    let range = self.range_to_utf16(&(0..self.value().len()));
                    self.replace_text_in_range_silent(Some(range), &new_value, window, cx);
                    return;
                }
            } else {
                return;
            }
        }
        cx.emit(NumberInputEvent::Step(action));
    }

    /// Sets the value to the minimum (or the maximum), as Home (or End)
    /// does in a number input. Returns `false`, changing nothing, when that
    /// bound is not set, so the key keeps moving the caret.
    fn apply_number_bound(
        &mut self,
        max: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let bound = if max {
            self.number_max
        } else {
            self.number_min
        };
        let Some(bound) = bound.filter(|_| !self.disabled) else {
            return false;
        };
        let text = bound.to_string();
        if self.unmask_value().as_ref() != text && self.is_valid_input(&text, cx) {
            let range = self.range_to_utf16(&(0..self.value().len()));
            self.replace_text_in_range_silent(Some(range), &text, window, cx);
        }
        true
    }
}

type StepHandler = Rc<dyn Fn(StepAction, &mut Window, &mut App)>;
type AmountStepHandler = Rc<dyn Fn(StepAction, StepAmount, &mut Window, &mut App)>;
type ButtonDecorator = Box<dyn FnOnce(Button) -> Button>;

/// An unstyled spinbutton root composed from the foundational [`InputBase`] frame.
///
/// Keyboard contract, from the focused editor (Base UI's Number Field):
///
/// - Up and Down step by the input state's step; Shift+Up and Shift+Down by
///   the large step ([`NumberInput::large_step`], 10 by default); Option+Up
///   and Option+Down by the small step ([`NumberInput::small_step`], 0.1 by
///   default).
/// - Home and End set the minimum and maximum, when the input state has one;
///   otherwise they move the caret as usual.
///
/// Pressing a step button steps once; holding it repeats the step after
/// 400 ms, then every 60 ms, until the button is released.
#[derive(IntoElement)]
pub struct NumberInput {
    style: StyleRefinement,
    children: Vec<AnyElement>,
    disabled: bool,
    state: Entity<InputState>,
    on_step: Option<StepHandler>,
    large_step: f64,
    small_step: f64,
    decrement_button: Option<ButtonDecorator>,
    increment_button: Option<ButtonDecorator>,
    input: Option<AnyElement>,
    controls_right: bool,
    accessibility_label: Option<SharedString>,
}

/// The built-in text region of a [`NumberInput`].
///
/// Applications provide the editor itself as a child and can style this region
/// without having to recreate the number input's fixed three-part structure.
#[derive(IntoElement)]
pub struct NumberInputText {
    style: StyleRefinement,
    children: Vec<AnyElement>,
}

impl NumberInputText {
    pub fn new() -> Self {
        Self {
            style: StyleRefinement::default(),
            children: Vec::new(),
        }
    }
}

impl Default for NumberInputText {
    fn default() -> Self {
        Self::new()
    }
}

impl Styled for NumberInputText {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for NumberInputText {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for NumberInputText {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        gpui::div()
            .min_w_0()
            .flex_1()
            .children(self.children)
            .refine_style(&self.style)
    }
}

impl NumberInput {
    pub fn new(state: &Entity<InputState>) -> Self {
        Self {
            style: StyleRefinement::default(),
            children: Vec::new(),
            disabled: false,
            state: state.clone(),
            on_step: None,
            large_step: 10.,
            small_step: 0.1,
            decrement_button: None,
            increment_button: None,
            input: None,
            controls_right: false,
            accessibility_label: None,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Takes over stepping: the handler receives every step request, from
    /// the keys and the buttons. Large and small steps reach it as ordinary
    /// steps in their direction.
    pub fn on_step(
        mut self,
        handler: impl Fn(StepAction, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_step = Some(Rc::new(handler));
        self
    }

    /// The amount Shift+Up and Shift+Down step by. Default: `10`.
    pub fn large_step(mut self, large_step: f64) -> Self {
        self.large_step = large_step;
        self
    }

    /// The amount Option+Up and Option+Down step by. Default: `0.1`.
    pub fn small_step(mut self, small_step: f64) -> Self {
        self.small_step = small_step;
        self
    }

    /// Decorate the built-in decrement button with application-owned content and styles.
    pub fn decrement_button(mut self, decorate: impl FnOnce(Button) -> Button + 'static) -> Self {
        self.decrement_button = Some(Box::new(decorate));
        self
    }

    /// Decorate the built-in increment button with application-owned content and styles.
    pub fn increment_button(mut self, decorate: impl FnOnce(Button) -> Button + 'static) -> Self {
        self.increment_button = Some(Box::new(decorate));
        self
    }

    /// Decorate the built-in text region with an editor, adornments, and styles.
    pub fn input(mut self, input: impl IntoElement) -> Self {
        self.input = Some(input.into_any_element());
        self
    }

    /// Stack both step buttons on the right side of the text region.
    pub fn controls_right(mut self) -> Self {
        self.controls_right = true;
        self
    }

    /// Sets the name exposed to accessibility clients.
    ///
    /// A number input has no visible text of its own to be named by, so set
    /// this unless the input is named some other way.
    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessibility_label = Some(label.into());
        self
    }
}

impl Styled for NumberInput {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for NumberInput {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for NumberInput {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let disabled = self.disabled;
        let controls_right = self.controls_right;
        let large_step = self.large_step;
        let small_step = self.small_step;
        let step: AmountStepHandler = match self.on_step {
            Some(on_step) => Rc::new(
                move |action: StepAction, _: StepAmount, window: &mut Window, cx: &mut App| {
                    on_step(action, window, cx)
                },
            ),
            None => {
                let state = self.state.clone();
                Rc::new(
                    move |action: StepAction,
                          amount: StepAmount,
                          window: &mut Window,
                          cx: &mut App| {
                        state.update(cx, |state, cx| {
                            state.focus(window, cx);
                            state.apply_number_step(action, amount, window, cx);
                        });
                    },
                )
            }
        };
        self.state.update(cx, |state, _| state.ensure_number_mask());
        let value = self.state.read(cx).value().parse::<f64>().ok();
        // The step a held button repeats. Replacing it cancels the one
        // before; dropping it on release stops the repeat.
        let repeat = window.use_keyed_state(
            ("number-input-repeat", self.state.entity_id()),
            cx,
            |_, _| None::<Task<()>>,
        );
        let decrement_button = self.decrement_button.map_or_else(
            || Button::new("decrement"),
            |decorate| decorate(Button::new("decrement")),
        );
        let increment_button = self.increment_button.map_or_else(
            || Button::new("increment"),
            |decorate| decorate(Button::new("increment")),
        );
        let text = NumberInputText::new().children(self.input);

        // Stepping is driven from the focused editor, so the buttons never take
        // focus themselves; otherwise every press would pull the focus ring off
        // the frame and hand it straight back. A press steps at once and a
        // held press repeats; the click that ends a mouse press is already
        // counted, so only clicks from elsewhere (assistive technology) step.
        let step_button = |button: Button, action: StepAction| {
            let press = {
                let step = step.clone();
                let repeat = repeat.clone();
                move |_: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                    step(action, StepAmount::Step, window, cx);
                    let step = step.clone();
                    let task = window.spawn(cx, async move |cx| {
                        cx.background_executor().timer(REPEAT_DELAY).await;
                        loop {
                            let stepped =
                                cx.update(|window, cx| step(action, StepAmount::Step, window, cx));
                            if stepped.is_err() {
                                break;
                            }
                            cx.background_executor().timer(REPEAT_INTERVAL).await;
                        }
                    });
                    repeat.update(cx, |repeat, _| *repeat = Some(task));
                }
            };
            let release = {
                let repeat = repeat.clone();
                move |_: &MouseUpEvent, _: &mut Window, cx: &mut App| {
                    repeat.update(cx, |repeat, _| *repeat = None);
                }
            };
            let click = {
                let step = step.clone();
                move |event: &ClickEvent, window: &mut Window, cx: &mut App| {
                    if !matches!(event, ClickEvent::Mouse(_)) {
                        step(action, StepAmount::Step, window, cx);
                    }
                }
            };
            button
                .when(controls_right, |this| this.flex_1().min_h_0())
                .when(!controls_right, |this| this.flex_none())
                .focusable(false)
                .disabled(disabled)
                .when(!disabled, |this| {
                    this.on_mouse_down(MouseButton::Left, press)
                        .on_mouse_up(MouseButton::Left, release.clone())
                        .on_mouse_up_out(MouseButton::Left, release)
                })
                .on_click(click)
        };
        let decrement_button = step_button(decrement_button, StepAction::Decrement);
        let increment_button = step_button(increment_button, StepAction::Increment);

        let content = if controls_right {
            div()
                .flex()
                .items_center()
                .size_full()
                .child(text.children(self.children))
                .child(
                    div()
                        .h_full()
                        .flex()
                        .flex_col()
                        .child(increment_button)
                        .child(decrement_button),
                )
                .into_any_element()
        } else {
            div()
                .flex()
                .items_center()
                .size_full()
                .child(decrement_button)
                .child(text.children(self.children))
                .child(increment_button)
                .into_any_element()
        };

        let key_step = |action: StepAction, amount: StepAmount| {
            let step = step.clone();
            move |window: &mut Window, cx: &mut App| {
                if disabled {
                    cx.propagate();
                } else {
                    step(action, amount, window, cx);
                }
            }
        };
        let increment = key_step(StepAction::Increment, StepAmount::Step);
        let decrement = key_step(StepAction::Decrement, StepAmount::Step);
        let increment_large = key_step(StepAction::Increment, StepAmount::By(large_step));
        let decrement_large = key_step(StepAction::Decrement, StepAmount::By(large_step));
        let increment_small = key_step(StepAction::Increment, StepAmount::By(small_step));
        let decrement_small = key_step(StepAction::Decrement, StepAmount::By(small_step));
        let bound = |max: bool| {
            let state = self.state.clone();
            move |window: &mut Window, cx: &mut App| {
                let applied =
                    state.update(cx, |state, cx| state.apply_number_bound(max, window, cx));
                // Home and End reach the editor first; take them only when
                // there is a bound to go to.
                if applied {
                    cx.stop_propagation();
                }
            }
        };
        let to_min = bound(false);
        let to_max = bound(true);

        InputBase::new(("number-input", self.state.entity_id()))
            .track_focus(&self.state.focus_handle(cx))
            .flex()
            .items_center()
            .disabled(disabled)
            .role(Role::SpinButton)
            .when_some(self.accessibility_label, |this, label| {
                this.aria_label(label)
            })
            .when_some(value, |this, value| this.aria_numeric_value(value))
            .key_context(CONTEXT)
            .on_action(move |_: &Increment, window, cx| increment(window, cx))
            .on_action(move |_: &Decrement, window, cx| decrement(window, cx))
            .on_action(move |_: &IncrementLarge, window, cx| increment_large(window, cx))
            .on_action(move |_: &DecrementLarge, window, cx| decrement_large(window, cx))
            .on_action(move |_: &IncrementSmall, window, cx| increment_small(window, cx))
            .on_action(move |_: &DecrementSmall, window, cx| decrement_small(window, cx))
            // The editor binds Home and End to caret moves in its own, deeper
            // key context, so they are caught on the way down to it.
            .capture_action(move |_: &MoveHome, window, cx| to_min(window, cx))
            .capture_action(move |_: &MoveEnd, window, cx| to_max(window, cx))
            .child(content)
            .refine_style(&self.style)
            .render(window, cx)
    }
}

/// Step a numeric string while preserving decimal precision and range direction.
pub fn step_value(
    value: &str,
    action: StepAction,
    step: f64,
    min: Option<f64>,
    max: Option<f64>,
) -> Option<String> {
    fn fraction_digits(value: &str) -> usize {
        value.split('.').nth(1).map_or(0, |fraction| fraction.len())
    }

    let current = value.trim().parse::<f64>().ok();
    let mut new_value = match action {
        StepAction::Increment => current.unwrap_or(0.) + step,
        StepAction::Decrement => current.unwrap_or(0.) - step,
    };
    let mut digits = fraction_digits(value).max(fraction_digits(&step.to_string()));
    if let Some(min) = min
        && new_value < min
    {
        new_value = min;
        digits = digits.max(fraction_digits(&min.to_string()));
    }
    if let Some(max) = max
        && new_value > max
    {
        new_value = max;
        digits = digits.max(fraction_digits(&max.to_string()));
    }

    if let Some(current) = current {
        let moved = match action {
            StepAction::Increment => new_value > current,
            StepAction::Decrement => new_value < current,
        };
        if !moved {
            return None;
        }
    }

    Some(format!("{new_value:.digits$}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::theme::Theme;
    use gpui::{
        AppContext as _, Context, Entity, Modifiers, MouseButton, Render, TestAppContext,
        VisualTestContext, point, px,
    };

    struct StepperHarness {
        state: Entity<InputState>,
    }

    impl Render for StepperHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            NumberInput::new(&self.state)
                .w(px(120.))
                .h(px(20.))
                .decrement_button(|button| button.w(px(20.)).h_full())
                .input(div().size_full())
                .increment_button(|button| button.w(px(20.)).h_full())
        }
    }

    /// Pressing a step button must not move focus off the editor, not even for
    /// the press itself: the frame draws a focus ring, so a focus round-trip
    /// makes it flicker on every click.
    #[gpui::test]
    fn pressing_a_step_button_never_takes_focus_off_the_editor(cx: &mut TestAppContext) {
        cx.update(crate::init);

        let mut created: Option<Entity<InputState>> = None;
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |window, cx| {
                cx.set_global(Theme::default());
                let state = cx.new(|cx| InputState::new(window, cx).step(1.));
                created = Some(state.clone());
                cx.new(|_| StepperHarness { state })
            })
            .unwrap()
        });
        let state = created.unwrap();
        let cx = &mut VisualTestContext::from_window(window.into(), cx);

        cx.update(|window, cx| {
            state.update(cx, |state, cx| state.focus(window, cx));
            window.draw(cx).clear(cx);
        });
        cx.update(|window, cx| {
            assert!(
                state.read(cx).focus_handle(cx).is_focused(window),
                "the editor should start focused"
            );
        });

        // The decrement button sits at the left edge of the control.
        cx.simulate_mouse_move(
            point(px(10.), px(10.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_event(gpui::MouseDownEvent {
            button: MouseButton::Left,
            position: point(px(10.), px(10.)),
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        });

        cx.update(|window, cx| {
            assert!(
                state.read(cx).focus_handle(cx).is_focused(window),
                "pressing the step button pulled focus off the editor"
            );
        });

        cx.simulate_event(gpui::MouseUpEvent {
            button: MouseButton::Left,
            position: point(px(10.), px(10.)),
            modifiers: Modifiers::default(),
            click_count: 1,
        });

        cx.update(|window, cx| {
            assert!(
                state.read(cx).focus_handle(cx).is_focused(window),
                "releasing the step button left focus off the editor"
            );
        });
    }

    /// A number input around a real editor, so the editor's own key bindings
    /// sit in front of the number input's, as in an application.
    struct EditorHarness {
        state: Entity<InputState>,
    }

    impl Render for EditorHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            NumberInput::new(&self.state)
                .w(px(120.))
                .h(px(20.))
                .decrement_button(|button| button.w(px(20.)).h_full())
                .input(crate::Input::new(&self.state))
                .increment_button(|button| button.w(px(20.)).h_full())
        }
    }

    fn editor_harness(
        cx: &mut TestAppContext,
        build: impl FnOnce(InputState) -> InputState + 'static,
    ) -> (Entity<InputState>, &mut VisualTestContext) {
        cx.update(crate::init);
        let mut created: Option<Entity<InputState>> = None;
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |window, cx| {
                cx.set_global(Theme::default());
                let state = cx.new(|cx| build(InputState::new(window, cx).default_value("50")));
                created = Some(state.clone());
                cx.new(|_| EditorHarness { state })
            })
            .unwrap()
        });
        let state = created.unwrap();
        let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
        cx.update(|window, cx| {
            state.update(cx, |state, cx| state.focus(window, cx));
            window.draw(cx).clear(cx);
        });
        (state, cx)
    }

    fn value(state: &Entity<InputState>, cx: &mut VisualTestContext) -> String {
        state.read_with(cx, |state, _| state.value().to_string())
    }

    #[gpui::test]
    fn shift_and_option_arrows_take_the_large_and_small_steps(cx: &mut TestAppContext) {
        let (state, cx) = editor_harness(cx, |state| state.step(1.));

        cx.simulate_keystrokes("up");
        assert_eq!(value(&state, cx), "51");
        cx.simulate_keystrokes("shift-up");
        assert_eq!(value(&state, cx), "61");
        cx.simulate_keystrokes("alt-down");
        assert_eq!(value(&state, cx), "60.9");
        cx.simulate_keystrokes("shift-down");
        assert_eq!(value(&state, cx), "50.9");
    }

    #[gpui::test]
    fn home_and_end_set_the_minimum_and_maximum(cx: &mut TestAppContext) {
        let (state, cx) = editor_harness(cx, |state| state.step(1.).min(0.).max(100.));

        cx.simulate_keystrokes("home");
        assert_eq!(value(&state, cx), "0");
        cx.simulate_keystrokes("end");
        assert_eq!(value(&state, cx), "100");
    }

    #[gpui::test]
    fn home_and_end_leave_the_value_alone_without_bounds(cx: &mut TestAppContext) {
        let (state, cx) = editor_harness(cx, |state| state.step(1.));

        cx.simulate_keystrokes("home end");
        assert_eq!(value(&state, cx), "50");
    }

    #[gpui::test]
    fn holding_a_step_button_repeats_the_step_until_it_is_released(cx: &mut TestAppContext) {
        let (state, cx) = editor_harness(cx, |state| state.step(1.));
        let at = point(px(10.), px(10.));
        // In ticks of 10 ms, so each repeat's timer starts when the last fired.
        let advance = |cx: &mut VisualTestContext, ms: u64| {
            for _ in 0..ms / 10 {
                cx.executor().advance_clock(Duration::from_millis(10));
                cx.run_until_parked();
            }
        };

        cx.simulate_mouse_move(at, MouseButton::Left, Modifiers::default());
        cx.simulate_event(gpui::MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        });
        assert_eq!(value(&state, cx), "49", "the press steps at once");
        advance(cx, 390);
        assert_eq!(value(&state, cx), "49", "no repeat before the delay");
        advance(cx, 20);
        assert_eq!(value(&state, cx), "48", "the first repeat after 400 ms");
        advance(cx, 120);
        assert_eq!(value(&state, cx), "46", "then one every 60 ms");

        cx.simulate_event(gpui::MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::default(),
            click_count: 1,
        });
        advance(cx, 500);
        assert_eq!(value(&state, cx), "46", "the release stops the repeat");
    }

    #[test]
    fn stepping_preserves_precision_and_directional_bounds() {
        assert_eq!(
            step_value("0.1", StepAction::Increment, 0.2, None, None).as_deref(),
            Some("0.3")
        );
        assert_eq!(
            step_value("10", StepAction::Decrement, 1., Some(10.), None),
            None
        );
        assert_eq!(
            step_value("99.5", StepAction::Increment, 1., None, Some(100.)).as_deref(),
            Some("100.0")
        );
        assert_eq!(
            step_value("10", StepAction::Increment, 1., None, Some(10.)),
            None
        );
        assert_eq!(
            step_value("5", StepAction::Decrement, 10., Some(0.), None).as_deref(),
            Some("0")
        );
    }
}
