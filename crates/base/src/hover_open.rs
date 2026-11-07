//! Opening a popup while the pointer rests on its trigger, and keeping it open
//! while the pointer travels to it: Base UI's hover with `safePolygon`, shared
//! by [`crate::HoverCard`] and a [`crate::Popover`] that opens on hover.
//!
//! The pointer resting on the trigger for the open delay opens the popup;
//! leaving both the trigger and the popup closes it after the close delay.
//! On the way from the trigger to the popup the close waits again from each
//! move toward it (see [`gpui::hover_intent`]), so a slow trip never closes it
//! and resting on the way, or turning away, does.

use std::time::Duration;

use gpui::{
    Bounds, Context, DispatchPhase, IntoElement, MouseMoveEvent, Pixels, Point, Styled as _, Task,
    WeakEntity, Window, canvas, hover_intent,
};

/// The least a trip from the trigger waits for the next move before the popup
/// closes: a popup that closes at once when left (a popover's default) still
/// lets the pointer cross to it, and closes once it rests, as floating-ui's
/// `safePolygon` does after 40 ms of no intent.
const TRIP_REST: Duration = Duration::from_millis(40);

/// An entity whose popup opens on hover.
pub(crate) trait HoverHost: 'static + Sized {
    fn hover(&mut self) -> &mut HoverOpen;

    /// The pointer asks the popup to open or close.
    fn hover_wants_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>);
}

/// Where the pointer is and the delay waiting to act on it.
pub(crate) struct HoverOpen {
    pub(crate) open_delay: Duration,
    pub(crate) close_delay: Duration,
    task: Option<Task<()>>,
    /// Bumped by each schedule and cancel, so a delay that outlived its
    /// reason does nothing.
    epoch: usize,
    over_trigger: bool,
    over_popup: bool,
    /// Where the pointer left the trigger while the popup was open, until it
    /// reaches the popup or turns away.
    apex: Option<Point<Pixels>>,
}

impl HoverOpen {
    pub(crate) fn new(open_delay: Duration, close_delay: Duration) -> Self {
        Self {
            open_delay,
            close_delay,
            task: None,
            epoch: 0,
            over_trigger: false,
            over_popup: false,
            apex: None,
        }
    }

    /// Drops the waiting delay and any trip in progress.
    pub(crate) fn cancel(&mut self) {
        self.epoch += 1;
        self.task = None;
        self.apex = None;
    }

    /// Whether the pointer is over the trigger.
    pub(crate) fn over_trigger(&self) -> bool {
        self.over_trigger
    }

    fn over_either(&self) -> bool {
        self.over_trigger || self.over_popup
    }
}

fn schedule<T: HoverHost>(host: &mut T, open: bool, window: &mut Window, cx: &mut Context<T>) {
    let hover = host.hover();
    hover.epoch += 1;
    let epoch = hover.epoch;
    let delay = if open {
        hover.open_delay
    } else if hover.apex.is_some() {
        hover.close_delay.max(TRIP_REST)
    } else {
        hover.close_delay
    };
    hover.task = Some(cx.spawn_in(window, async move |this, cx| {
        cx.background_executor().timer(delay).await;
        _ = this.update_in(cx, |host, window, cx| {
            let hover = host.hover();
            if hover.epoch != epoch || (!open && hover.over_either()) {
                return;
            }
            hover.task = None;
            host.hover_wants_open(open, window, cx);
        });
    }));
}

/// The pointer entered or left the trigger of a popup that is `open` or not.
pub(crate) fn trigger_hover<T: HoverHost>(
    host: &mut T,
    hovering: bool,
    open: bool,
    window: &mut Window,
    cx: &mut Context<T>,
) {
    let hover = host.hover();
    hover.over_trigger = hovering;
    hover.apex = None;
    if hovering {
        schedule(host, true, window, cx);
    } else if !hover.over_popup {
        if open {
            hover.apex = Some(window.mouse_position());
        }
        schedule(host, false, window, cx);
    }
}

/// The pointer entered or left the popup.
pub(crate) fn popup_hover<T: HoverHost>(
    host: &mut T,
    hovering: bool,
    window: &mut Window,
    cx: &mut Context<T>,
) {
    let hover = host.hover();
    hover.over_popup = hovering;
    if hovering {
        hover.cancel();
    } else if !hover.over_trigger {
        hover.apex = None;
        schedule(host, false, window, cx);
    }
}

/// The pointer moved while the popup at `popup` was open: on its way from the
/// trigger, the close waits again from now; once it turns away, the close
/// already waiting runs out.
fn travel<T: HoverHost>(
    host: &mut T,
    popup: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut Context<T>,
) {
    let hover = host.hover();
    let Some(apex) = hover.apex else { return };
    if hover_intent::heading_to(apex, popup, window.mouse_position()) {
        schedule(host, false, window, cx);
    } else {
        hover.apex = None;
    }
}

/// An element over the popup's bounds that follows the pointer on its way
/// from the trigger. Its parent must be `relative`.
pub(crate) fn travel_watcher<T: HoverHost>(host: WeakEntity<T>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |popup, _, window, _| {
            let host = host.clone();
            window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture {
                    host.update(cx, |host, cx| travel(host, popup, window, cx))
                        .ok();
                }
            });
        },
    )
    .absolute()
    .size_full()
}
