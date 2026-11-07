use std::rc::Rc;

use gpui::{
    Anchor, AnyElement, App, Context, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, RenderOnce, Stateful, StatefulInteractiveElement as _,
    StyleRefinement, Styled, Subscription, Window, div, prelude::FluentBuilder as _,
};
use std::time::Duration;

use crate::{
    Popup, StyledExt as _,
    hover_open::{self, HoverHost, HoverOpen},
};

type ContentBuilder = Box<
    dyn FnOnce(
        &mut HoverCardState,
        &mut Window,
        &mut Context<HoverCardState>,
    ) -> Stateful<gpui::Div>,
>;
type OpenChangeHandler = Rc<dyn Fn(&bool, &mut Window, &mut App)>;

/// An unstyled popup with delayed hover behavior on desktop.
/// On iOS, tapping toggles it and tapping outside dismisses it.
#[derive(IntoElement)]
pub struct HoverCard {
    id: ElementId,
    style: StyleRefinement,
    anchor: Anchor,
    tap_to_open: bool,
    trigger: Option<AnyElement>,
    content: Option<ContentBuilder>,
    open_delay: Duration,
    close_delay: Duration,
    on_open_change: Option<OpenChangeHandler>,
}

impl HoverCard {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            style: StyleRefinement::default(),
            anchor: Anchor::TopCenter,
            tap_to_open: crate::is_mobile(),
            trigger: None,
            content: None,
            open_delay: Duration::from_secs_f64(0.6),
            close_delay: Duration::from_secs_f64(0.3),
            on_open_change: None,
        }
    }

    pub fn anchor(mut self, anchor: impl Into<Anchor>) -> Self {
        self.anchor = anchor.into();
        self
    }

    pub fn trigger(mut self, trigger: impl IntoElement) -> Self {
        self.trigger = Some(trigger.into_any_element());
        self
    }

    pub fn content<F>(mut self, content: F) -> Self
    where
        F: FnOnce(
                &mut HoverCardState,
                &mut Window,
                &mut Context<HoverCardState>,
            ) -> Stateful<gpui::Div>
            + 'static,
    {
        self.content = Some(Box::new(content));
        self
    }

    pub fn open_delay(mut self, duration: Duration) -> Self {
        self.open_delay = duration;
        self
    }

    pub fn close_delay(mut self, duration: Duration) -> Self {
        self.close_delay = duration;
        self
    }

    pub fn on_open_change(
        mut self,
        callback: impl Fn(&bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open_change = Some(Rc::new(callback));
        self
    }
}

/// Styles the trigger container: the element that takes part in the parent
/// layout and whose bounds the card is anchored to.
impl Styled for HoverCard {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

/// State exposed to a [`HoverCard::content`] builder.
pub struct HoverCardState {
    open: bool,
    hover: HoverOpen,
    on_open_change: Option<OpenChangeHandler>,
    /// While open: Escape, from any focus in its window, puts it away.
    escape: Option<Subscription>,
}

impl HoverCardState {
    fn new(open_delay: Duration, close_delay: Duration) -> Self {
        Self {
            open: false,
            hover: HoverOpen::new(open_delay, close_delay),
            on_open_change: None,
            escape: None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    fn sync(
        &mut self,
        open_delay: Duration,
        close_delay: Duration,
        on_open_change: Option<OpenChangeHandler>,
    ) {
        self.hover.open_delay = open_delay;
        self.hover.close_delay = close_delay;
        self.on_open_change = on_open_change;
    }

    fn set_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.open == open {
            return;
        }

        self.open = open;
        self.escape = open.then(|| {
            let card = cx.entity().downgrade();
            let home = window.window_handle();
            cx.intercept_keystrokes(move |event, window, cx| {
                let key = &event.keystroke;
                if key.key == "escape"
                    && !key.modifiers.modified()
                    && window.window_handle() == home
                {
                    card.update(cx, |state, cx| state.dismiss(window, cx)).ok();
                }
            })
        });
        cx.notify();
        // The change is announced from here rather than from the element,
        // because the delay timers outlive the `HoverCard` that carried the
        // handler: by the time the state flips, that element is long gone.
        if let Some(on_open_change) = self.on_open_change.clone() {
            on_open_change(&open, window, cx);
        }
    }

    /// Escape puts an open card away, as Base UI's does; it opens again once
    /// the pointer comes back to the trigger.
    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.hover.cancel();
        self.set_open(false, window, cx);
    }
}

impl HoverHost for HoverCardState {
    fn hover(&mut self) -> &mut HoverOpen {
        &mut self.hover
    }

    fn hover_wants_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.set_open(open, window, cx);
    }
}

impl Render for HoverCardState {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl RenderOnce for HoverCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| {
            HoverCardState::new(self.open_delay, self.close_delay)
        });
        state.update(cx, |state, _| {
            state.sync(self.open_delay, self.close_delay, self.on_open_change)
        });
        let open = state.read(cx).is_open();

        let trigger = self.trigger.unwrap_or_else(|| div().into_any_element());
        let popup = Popup::new(
            self.id,
            div()
                .id("trigger")
                .child(trigger)
                .when(self.tap_to_open, |trigger| {
                    trigger.on_click(window.listener_for(&state, move |state, _, window, cx| {
                        state.hover.cancel();
                        // Toggle the state rendered by this trigger even if
                        // outside dismissal handles the same release first.
                        state.set_open(!open, window, cx);
                    }))
                })
                .when(!self.tap_to_open, |trigger| {
                    trigger.on_hover(window.listener_for(&state, |state, hovered, window, cx| {
                        let open = state.open;
                        hover_open::trigger_hover(state, *hovered, open, window, cx)
                    }))
                }),
        )
        .refine_style(&self.style)
        .anchor(self.anchor);

        if !open {
            return popup;
        }

        popup.when_some(self.content, |popup, content| {
            let hover = window.listener_for(&state, |state, hovered, window, cx| {
                hover_open::popup_hover(state, *hovered, window, cx)
            });
            let dismiss = window.listener_for(&state, |state, _, window, cx| {
                state.hover.cancel();
                state.set_open(false, window, cx);
            });
            // Over the card's bounds: follows the pointer on its way from the
            // trigger.
            let watcher = hover_open::travel_watcher(state.downgrade());
            popup.content(state.update(cx, |state, cx| {
                content(state, window, cx)
                    .when(self.tap_to_open, |content| {
                        content.on_mouse_up_out(gpui::MouseButton::Left, dismiss)
                    })
                    .when(!self.tap_to_open, |content| {
                        content.on_hover(hover).relative().child(watcher)
                    })
            }))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui::{Context, Render, TestAppContext, point, px};

    use super::*;

    #[derive(Default)]
    struct Harness {
        open_changes: Rc<RefCell<Vec<bool>>>,
        tap_to_open: bool,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let delay = Duration::from_millis(100);
            let open_changes = self.open_changes.clone();
            HoverCard::new("hover-card")
                .map(|mut card| {
                    card.tap_to_open = self.tap_to_open;
                    card
                })
                .open_delay(delay)
                .close_delay(delay)
                .on_open_change(move |open, _, _| open_changes.borrow_mut().push(*open))
                .trigger(
                    div()
                        .debug_selector(|| "hover-card-trigger".into())
                        .size(px(20.)),
                )
                .content(|_, _, _| {
                    div()
                        .id("hover-card-content")
                        .debug_selector(|| "hover-card-content".into())
                        .size(px(10.))
                })
        }
    }

    #[gpui::test]
    fn public_hover_card_owns_delayed_open_and_close(cx: &mut TestAppContext) {
        let delay = Duration::from_millis(100);
        let (_, cx) = cx.add_window_view(|_, _| Harness::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));

        cx.simulate_mouse_move(point(px(10.), px(10.)), None, gpui::Modifiers::default());
        cx.executor().advance_clock(delay);
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("hover-card-content").is_some());

        cx.simulate_mouse_move(point(px(100.), px(100.)), None, gpui::Modifiers::default());
        cx.executor().advance_clock(delay);
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("hover-card-content").is_none());
    }

    /// Opens the harness's card by hovering its trigger, and returns the
    /// trigger's and the card's bounds.
    fn open_card(
        cx: &mut gpui::VisualTestContext,
    ) -> (gpui::Bounds<gpui::Pixels>, gpui::Bounds<gpui::Pixels>) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_mouse_move(point(px(10.), px(10.)), None, gpui::Modifiers::default());
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            window.draw(cx).clear(cx);
        });
        let trigger = cx.debug_bounds("hover-card-trigger").unwrap();
        let card = cx.debug_bounds("hover-card-content").unwrap();
        (trigger, card)
    }

    fn settle(cx: &mut gpui::VisualTestContext, ms: u64) {
        cx.executor().advance_clock(Duration::from_millis(ms));
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    /// A slow trip from the trigger to the card, longer than the close delay
    /// in all, keeps the card while the pointer heads for it.
    #[gpui::test]
    fn a_slow_trip_to_the_card_keeps_it_open(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| Harness::default());
        let (trigger, card) = open_card(cx);
        assert!(
            card.top() >= trigger.bottom(),
            "the card opens below its trigger"
        );
        let start = point(card.center().x, trigger.bottom() - px(1.));
        let end = card.center();
        for step in 1..=5 {
            let t = step as f32 / 6.;
            let at = point(
                start.x + (end.x - start.x) * t,
                start.y + (end.y - start.y) * t,
            );
            cx.simulate_mouse_move(at, None, gpui::Modifiers::default());
            settle(cx, 60);
            assert!(
                cx.debug_bounds("hover-card-content").is_some(),
                "closed at step {step}"
            );
        }
        cx.simulate_mouse_move(end, None, gpui::Modifiers::default());
        settle(cx, 300);
        assert!(cx.debug_bounds("hover-card-content").is_some());
    }

    /// Leaving the trigger away from the card closes it after the close delay,
    /// even when the pointer keeps moving.
    #[gpui::test]
    fn turning_away_from_the_card_closes_it(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| Harness::default());
        let (_, card) = open_card(cx);
        // Below and right of the card, then farther: the trigger's row spans the
        // window, so the pointer leaves it downward.
        let away = card.bottom_right() + point(px(90.), px(30.));
        cx.simulate_mouse_move(away, None, gpui::Modifiers::default());
        settle(cx, 60);
        cx.simulate_mouse_move(
            away + point(px(20.), px(20.)),
            None,
            gpui::Modifiers::default(),
        );
        settle(cx, 60);
        assert!(cx.debug_bounds("hover-card-content").is_none());
    }

    #[gpui::test]
    fn escape_puts_an_open_card_away(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| Harness::default());
        open_card(cx);
        cx.simulate_keystrokes("escape");
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("hover-card-content").is_none());
        // Still over the trigger, it stays away until the pointer comes back.
        settle(cx, 300);
        assert!(cx.debug_bounds("hover-card-content").is_none());
    }

    #[gpui::test]
    fn public_hover_card_reports_each_open_change(cx: &mut TestAppContext) {
        let delay = Duration::from_millis(100);
        let open_changes = Rc::new(RefCell::new(Vec::new()));
        let (_, cx) = cx.add_window_view({
            let open_changes = open_changes.clone();
            move |_, _| Harness {
                open_changes,
                ..Default::default()
            }
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));

        cx.simulate_mouse_move(point(px(10.), px(10.)), None, gpui::Modifiers::default());
        assert_eq!(*open_changes.borrow(), Vec::<bool>::new());

        cx.executor().advance_clock(delay);
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            window.draw(cx).clear(cx);
        });
        assert_eq!(*open_changes.borrow(), vec![true]);

        cx.simulate_mouse_move(point(px(100.), px(100.)), None, gpui::Modifiers::default());
        cx.executor().advance_clock(delay);
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(*open_changes.borrow(), vec![true, false]);
    }
    #[gpui::test]
    fn tap_card_ignores_hover_and_toggles_and_dismisses(cx: &mut TestAppContext) {
        let changes = Rc::new(RefCell::new(Vec::new()));
        let (_, cx) = cx.add_window_view({
            let changes = changes.clone();
            move |_, _| Harness {
                tap_to_open: true,
                open_changes: changes,
            }
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_mouse_move(point(px(10.), px(10.)), None, Default::default());
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        assert!(changes.borrow().is_empty());

        cx.simulate_click(point(px(10.), px(10.)), Default::default());
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("hover-card-content").is_some());
        cx.simulate_mouse_move(point(px(100.), px(100.)), None, Default::default());
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        assert_eq!(*changes.borrow(), vec![true]);

        cx.simulate_click(point(px(10.), px(10.)), Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("hover-card-content").is_none());
        assert_eq!(*changes.borrow(), vec![true, false]);

        cx.simulate_click(point(px(10.), px(10.)), Default::default());
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            window.draw(cx).clear(cx);
        });
        let bounds = cx.debug_bounds("hover-card-content").unwrap();
        cx.simulate_click(bounds.center(), Default::default());
        assert_eq!(*changes.borrow(), vec![true, false, true]);
        cx.simulate_click(point(px(100.), px(100.)), Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("hover-card-content").is_none());
        assert_eq!(*changes.borrow(), vec![true, false, true, false]);
    }
}
