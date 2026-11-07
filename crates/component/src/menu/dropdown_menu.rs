use std::rc::Rc;

use gpui::{
    Anchor, AnyElement, App, Context, DismissEvent, Element, ElementId, Entity, FocusHandle,
    Focusable, GlobalElementId, InspectorElementId, InteractiveElement, IntoElement, KeyBinding,
    LayoutId, MouseButton, RenderOnce, SharedString, Styled, Window, prelude::FluentBuilder,
};

use crate::{
    Selectable,
    actions::{Confirm, SelectDown, SelectUp},
    button::Button,
    menu::{PopupMenu, popup_menu::MenuEdge},
    popover::Popover,
};

/// The key context of a dropdown menu's trigger.
const TRIGGER_CONTEXT: &str = "DropdownMenuTrigger";

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("down", SelectDown, Some(TRIGGER_CONTEXT)),
        KeyBinding::new("up", SelectUp, Some(TRIGGER_CONTEXT)),
    ]);
}

/// A dropdown menu trait for buttons and other interactive elements
pub trait DropdownMenu: Styled + Selectable + InteractiveElement + IntoElement + 'static {
    /// Create a dropdown menu with the given items, anchored to the TopLeft corner
    fn dropdown_menu(
        self,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> DropdownMenuPopover<Self> {
        self.dropdown_menu_with_anchor(Anchor::TopLeft, f)
    }

    /// Create a dropdown menu with the given items, anchored to the given corner
    fn dropdown_menu_with_anchor(
        mut self,
        anchor: impl Into<Anchor>,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> DropdownMenuPopover<Self> {
        let id = self.interactivity().element_id.clone();

        DropdownMenuPopover::new(id.unwrap_or(0.into()), anchor, self, f)
    }
}

impl DropdownMenu for Button {}

#[derive(IntoElement)]
pub struct DropdownMenuPopover<T: Selectable + InteractiveElement + IntoElement + 'static> {
    id: ElementId,
    anchor: Anchor,
    trigger: T,
    builder: Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>,
    on_open_change: Option<Rc<dyn Fn(&bool, &mut Window, &mut App)>>,
}

impl<T> DropdownMenuPopover<T>
where
    T: Selectable + InteractiveElement + IntoElement + 'static,
{
    fn new(
        id: ElementId,
        anchor: impl Into<Anchor>,
        trigger: T,
        builder: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        Self {
            id: SharedString::from(format!("dropdown-menu:{:?}", id)).into(),
            anchor: anchor.into(),
            trigger,
            builder: Rc::new(builder),
            on_open_change: None,
        }
    }

    /// Set the anchor corner for the dropdown menu popover.
    pub fn anchor(mut self, anchor: impl Into<Anchor>) -> Self {
        self.anchor = anchor.into();
        self
    }

    /// Add a callback to be called when the menu opens or closes.
    ///
    /// The `&bool` parameter is the **new open state**.
    pub fn on_open_change(
        mut self,
        callback: impl Fn(&bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open_change = Some(Rc::new(callback));
        self
    }
}

#[derive(Default)]
struct DropdownMenuState {
    menu: Option<Entity<PopupMenu>>,
    /// The item to highlight as the menu opens from an arrow key.
    open_highlight: Option<MenuEdge>,
}

/// An arrow key on the closed trigger opens the menu with its first or last
/// item highlighted, as Base UI's menu trigger does.
fn open_from_arrow_key(
    menu_state: &Entity<DropdownMenuState>,
    edge: MenuEdge,
    window: &mut Window,
    cx: &mut App,
) {
    if menu_state.read(cx).menu.is_some() {
        return;
    }
    menu_state.update(cx, |state, _| state.open_highlight = Some(edge));
    // The popover opens on `Confirm` from its trigger, as on Enter.
    window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
}

impl<T> RenderOnce for DropdownMenuPopover<T>
where
    T: Selectable + InteractiveElement + IntoElement + 'static,
{
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        TriggerFocus::new(self.id.clone(), move |trigger_focus, window, cx| {
            self.render_popover(trigger_focus, window, cx)
                .into_any_element()
        })
    }
}

impl<T> DropdownMenuPopover<T>
where
    T: Selectable + InteractiveElement + IntoElement + 'static,
{
    fn render_popover(
        self,
        trigger_focus: FocusHandle,
        window: &mut Window,
        cx: &mut App,
    ) -> Popover {
        let builder = self.builder.clone();
        let menu_state =
            window.use_keyed_state(self.id.clone(), cx, |_, _| DropdownMenuState::default());

        // The trigger has no key context of its own, so this one is safe to
        // give it: an arrow key there opens the menu.
        let trigger = self
            .trigger
            .key_context(TRIGGER_CONTEXT)
            .on_action({
                let menu_state = menu_state.clone();
                move |_: &SelectDown, window, cx| {
                    open_from_arrow_key(&menu_state, MenuEdge::First, window, cx)
                }
            })
            .on_action({
                let menu_state = menu_state.clone();
                move |_: &SelectUp, window, cx| {
                    open_from_arrow_key(&menu_state, MenuEdge::Last, window, cx)
                }
            });

        Popover::new(SharedString::from(format!("popover:{}", self.id)))
            .appearance(false)
            .overlay_closable(false)
            .trigger(trigger)
            .anchor(self.anchor)
            .when_some(self.on_open_change, |this, callback| {
                this.on_open_change(move |open, window, cx| callback(open, window, cx))
            })
            .content(move |_, window, cx| {
                // Here is special logic to only create the PopupMenu once and reuse it.
                // Because this `content` will called in every time render, so we need to store the menu
                // in state to avoid recreating at every render.
                //
                // And we also need to rebuild the menu when it is dismissed, to rebuild menu items
                // dynamically for support `dropdown_menu` method, so we listen for DismissEvent below.
                let menu = match menu_state.read(cx).menu.clone() {
                    Some(menu) => {
                        // An option was set with the menu left open: build its
                        // items again from this render's builder, which sees
                        // the new state.
                        if menu.read(cx).is_stale() {
                            let builder = builder.clone();
                            menu.update(cx, |menu, cx| {
                                menu.rebuild_stale(window, cx, move |menu, window, cx| {
                                    builder(menu, window, cx)
                                })
                            });
                        }
                        menu
                    }
                    None => {
                        let builder = builder.clone();
                        let menu = PopupMenu::build(window, cx, move |menu, window, cx| {
                            builder(menu, window, cx)
                        });
                        // Opened from the keyboard, the menu starts with an
                        // item highlighted: the last for the up arrow, else
                        // the first. Opened by a press, releasing it over an
                        // item picks that item.
                        let from_keyboard = window.last_input_was_keyboard();
                        let pressed = !from_keyboard && !window.last_input_was_touch();
                        let highlight = menu_state
                            .update(cx, |state, _| state.open_highlight.take())
                            .or(from_keyboard.then_some(MenuEdge::First));
                        menu.update(cx, |menu, cx| {
                            menu.set_trigger_focus(Some(trigger_focus.clone()), cx);
                            if let Some(edge) = highlight {
                                menu.highlight_edge(edge, cx);
                            }
                            if pressed {
                                menu.arm_release(MouseButton::Left);
                            }
                        });
                        menu_state.update(cx, |state, _| {
                            state.menu = Some(menu.clone());
                        });
                        menu.focus_handle(cx).focus(window, cx);

                        // Listen for dismiss events from the PopupMenu to close the popover.
                        //
                        // Hold Weak handles here, not strong clones: the listener
                        // lives as long as `menu`, which `menu_state` owns, so a
                        // strong capture would close the cycle
                        // `menu_state -> menu -> listener -> menu_state` and leak
                        // the `PopupMenu` and `DropdownMenuState` when the trigger
                        // stops being rendered while the menu is open.
                        let popover_state = cx.entity().downgrade();
                        window
                            .subscribe(&menu, cx, {
                                let menu_state = menu_state.downgrade();
                                move |_, _: &DismissEvent, window, cx| {
                                    if let Some(popover_state) = popover_state.upgrade() {
                                        popover_state.update(cx, |state, cx| {
                                            state.dismiss(window, cx);
                                        });
                                    }
                                    _ = menu_state.update(cx, |state, _| {
                                        state.menu = None;
                                        state.open_highlight = None;
                                    });
                                }
                            })
                            .detach();

                        menu.clone()
                    }
                };

                menu.clone()
            })
    }
}

type TriggerFocusBuild = Box<dyn FnOnce(FocusHandle, &mut Window, &mut App) -> AnyElement>;

/// Registers a focus handle on the trigger's dispatch node without ever
/// focusing it, so the menu opened from the trigger can resolve its shortcut
/// hints against the trigger's key contexts on the frame it opens. GPUI looks
/// a handle up in the previously rendered frame; the trigger was in it when
/// the menu was not yet.
struct TriggerFocus {
    id: ElementId,
    build: Option<TriggerFocusBuild>,
}

#[derive(Default)]
struct TriggerFocusState {
    focus_handle: Option<FocusHandle>,
}

struct TriggerFocusFrame {
    focus_handle: FocusHandle,
    child: AnyElement,
}

impl TriggerFocus {
    fn new(
        id: ElementId,
        build: impl FnOnce(FocusHandle, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        Self {
            id,
            build: Some(Box::new(build)),
        }
    }
}

impl IntoElement for TriggerFocus {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TriggerFocus {
    type RequestLayoutState = TriggerFocusFrame;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let focus_handle =
            window.with_optional_element_state::<TriggerFocusState, _>(id, |state, _| {
                let mut state = state.flatten().unwrap_or_default();
                let focus_handle = state
                    .focus_handle
                    .get_or_insert_with(|| cx.focus_handle())
                    .clone();
                (focus_handle, Some(state))
            });
        let build = self.build.take().expect("TriggerFocus is laid out once");
        let mut child = build(focus_handle.clone(), window, cx);
        let layout_id = child.request_layout(window, cx);

        (
            layout_id,
            TriggerFocusFrame {
                focus_handle,
                child,
            },
        )
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: gpui::Bounds<gpui::Pixels>,
        frame: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.set_focus_handle(&frame.focus_handle, cx);
        frame.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: gpui::Bounds<gpui::Pixels>,
        frame: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        frame.child.paint(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{ParentElement as _, Render, TestAppContext, WeakEntity, actions, div, point, px};
    use std::cell::{Cell, RefCell};

    actions!(
        dropdown_menu_test,
        [CopyText, CutText, PasteText, ToggleWrap]
    );

    const CONTEXT: &str = "dropdown_menu_test";

    /// The story shape: the key binding lives in the key context of the
    /// trigger's ancestor, the menu names no `action_context`, and other
    /// content outside that context paints after the trigger.
    struct TestRoot {
        frames: Rc<Cell<usize>>,
    }

    impl Render for TestRoot {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.frames.set(self.frames.get() + 1);
            div()
                .size_full()
                .child(
                    div()
                        .key_context(CONTEXT)
                        .on_action(|_: &CopyText, _, _| {})
                        .child(
                            Button::new("trigger")
                                .label("Edit")
                                .w(px(100.))
                                .h(px(30.))
                                .dropdown_menu(|menu, _, _| menu.menu("Copy", Box::new(CopyText))),
                        ),
                )
                .child(div().child("Status"))
        }
    }

    /// Records the `PopupMenu` the dropdown builds, so a test can observe
    /// its release through a weak handle.
    struct MenuProbeRoot {
        menu: Rc<RefCell<Option<WeakEntity<PopupMenu>>>>,
    }

    impl Render for MenuProbeRoot {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let slot = self.menu.clone();
            div().size_full().child(
                Button::new("trigger")
                    .label("Edit")
                    .w(px(100.))
                    .h(px(30.))
                    .dropdown_menu(move |menu, _, cx| {
                        *slot.borrow_mut() = Some(cx.weak_entity());
                        menu.menu("Copy", Box::new(CopyText))
                    }),
            )
        }
    }

    /// Opening the menu and closing the window without dismissing it must
    /// release the `PopupMenu`: the dismiss subscription lives as long as the
    /// menu, which `DropdownMenuState` owns, so strong captures in it formed
    /// a cycle that no element-state collection could break. The state still
    /// holds the menu while it is open, so the menu's release also proves the
    /// `DropdownMenuState` was released.
    ///
    /// The popover must go too: it holds the deferred-popover registration
    /// while open, and a leaked one keeps every later right-click menu
    /// stepping aside as if a popup were still showing.
    #[gpui::test]
    fn open_without_dismiss_releases_the_menu(cx: &mut TestAppContext) {
        cx.update(|cx| crate::init(cx));
        let menu = Rc::new(RefCell::new(None::<WeakEntity<PopupMenu>>));

        {
            let (_, cx) = cx.add_window_view(|_, _| MenuProbeRoot { menu: menu.clone() });
            cx.update(|window, cx| window.draw(cx).clear(cx));

            // Click the trigger; the menu opens and is left open.
            cx.simulate_mouse_down(
                point(px(10.), px(10.)),
                MouseButton::Left,
                Default::default(),
            );
            cx.run_until_parked();
            assert!(
                menu.borrow()
                    .as_ref()
                    .and_then(|menu| menu.upgrade())
                    .is_some(),
                "the menu must be open before the window closes"
            );

            // Close the window without dismissing the menu.
            cx.update(|window, _| window.remove_window());
            cx.run_until_parked();
        }

        assert!(
            menu.borrow()
                .as_ref()
                .and_then(|menu| menu.upgrade())
                .is_none(),
            "the PopupMenu must be released with the window"
        );
        cx.update(|cx| {
            assert!(
                !gpui_base::GlobalState::is_in_deferred_context(cx),
                "the popover's deferred registration must be released with the window"
            )
        });
    }

    #[gpui::test]
    fn shortcut_hint_is_painted_on_the_frame_the_menu_opens(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::init(cx);
            cx.bind_keys([KeyBinding::new("ctrl-c", CopyText, Some(CONTEXT))]);
        });
        let frames = Rc::new(Cell::new(0));
        let (_, cx) = cx.add_window_view({
            let frames = frames.clone();
            move |_, _| TestRoot { frames }
        });
        // The popup host captures its trigger bounds on the first frame.
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let frames_before_open = frames.get();

        cx.simulate_mouse_down(
            point(px(10.), px(10.)),
            MouseButton::Left,
            Default::default(),
        );

        assert_eq!(
            frames.get(),
            frames_before_open + 1,
            "the press must be followed by exactly one frame for this to test the first one"
        );
        assert!(
            cx.debug_bounds("kbd:ctrl-c").is_some(),
            "the shortcut hint must be painted on the same frame as its item"
        );
    }

    /// A press on the trigger of an open menu closes it, as a native popup
    /// button's does: the press that dismisses the menu from outside its surface
    /// must not reach the trigger and open it again on the same click.
    #[gpui::test]
    fn a_press_on_the_trigger_of_an_open_menu_closes_it(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::init(cx);
            cx.bind_keys([KeyBinding::new("ctrl-c", CopyText, Some(CONTEXT))]);
        });
        let frames = Rc::new(Cell::new(0));
        let (_, cx) = cx.add_window_view({
            let frames = frames.clone();
            move |_, _| TestRoot { frames }
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let press = |cx: &mut gpui::VisualTestContext| {
            let at = point(px(10.), px(10.));
            cx.simulate_mouse_down(at, MouseButton::Left, Default::default());
            cx.simulate_mouse_up(at, MouseButton::Left, Default::default());
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
        };

        press(cx);
        assert!(
            cx.debug_bounds("kbd:ctrl-c").is_some(),
            "the first press opens the menu"
        );
        press(cx);
        assert!(
            cx.debug_bounds("kbd:ctrl-c").is_none(),
            "a press on the trigger closes the open menu"
        );
        press(cx);
        assert!(
            cx.debug_bounds("kbd:ctrl-c").is_some(),
            "and the next opens it again"
        );
    }

    /// A focusable trigger whose menu has three items, recording the menu.
    struct KeyboardRoot {
        trigger_focus: FocusHandle,
        menu: Rc<RefCell<Option<WeakEntity<PopupMenu>>>>,
    }

    impl Render for KeyboardRoot {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let slot = self.menu.clone();
            div().size_full().child(
                Button::new("trigger")
                    .label("Edit")
                    .w(px(100.))
                    .h(px(30.))
                    .track_focus(&self.trigger_focus)
                    .dropdown_menu(move |menu, _, cx| {
                        *slot.borrow_mut() = Some(cx.weak_entity());
                        menu.menu("Copy", Box::new(CopyText))
                            .menu("Cut", Box::new(CutText))
                            .menu("Paste", Box::new(PasteText))
                    }),
            )
        }
    }

    /// Focuses the trigger, types `keys`, and returns the opened menu's
    /// highlighted item.
    fn highlight_after_keys(keys: &str, cx: &mut TestAppContext) -> Option<usize> {
        cx.update(|cx| crate::init(cx));
        let menu = Rc::new(RefCell::new(None::<WeakEntity<PopupMenu>>));
        let (view, cx) = cx.add_window_view({
            let menu = menu.clone();
            move |_, cx| KeyboardRoot {
                trigger_focus: cx.focus_handle(),
                menu,
            }
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| {
            let focus = view.read(cx).trigger_focus.clone();
            focus.focus(window, cx);
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));

        cx.simulate_keystrokes(keys);
        cx.update(|window, cx| window.draw(cx).clear(cx));

        let menu = menu
            .borrow()
            .as_ref()
            .and_then(|menu| menu.upgrade())
            .expect("the key opens the menu");
        menu.read_with(cx, |menu, _| menu.highlighted())
    }

    #[gpui::test]
    fn arrow_down_on_the_trigger_opens_the_menu_with_its_first_item_highlighted(
        cx: &mut TestAppContext,
    ) {
        assert_eq!(highlight_after_keys("down", cx), Some(0));
    }

    #[gpui::test]
    fn arrow_up_on_the_trigger_opens_the_menu_with_its_last_item_highlighted(
        cx: &mut TestAppContext,
    ) {
        assert_eq!(highlight_after_keys("up", cx), Some(2));
    }

    #[gpui::test]
    fn enter_on_the_trigger_opens_the_menu_with_its_first_item_highlighted(
        cx: &mut TestAppContext,
    ) {
        assert_eq!(highlight_after_keys("enter", cx), Some(0));
    }

    #[gpui::test]
    fn space_on_the_trigger_opens_the_menu_with_its_first_item_highlighted(
        cx: &mut TestAppContext,
    ) {
        assert_eq!(highlight_after_keys("space", cx), Some(0));
    }

    /// A menu with one item, counting the times it is picked.
    struct PickRoot {
        picked: Rc<Cell<usize>>,
    }

    impl Render for PickRoot {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let picked = self.picked.clone();
            div().size_full().child(
                Button::new("trigger")
                    .label("Edit")
                    .w(px(100.))
                    .h(px(30.))
                    .dropdown_menu(move |menu, _, _| {
                        let picked = picked.clone();
                        menu.item(
                            crate::menu::PopupMenuItem::element(|_, _| {
                                div()
                                    .debug_selector(|| "pick-target".into())
                                    .w(px(80.))
                                    .h(px(20.))
                            })
                            .on_click(move |_, _, _| picked.set(picked.get() + 1)),
                        )
                    }),
            )
        }
    }

    /// As in a native menu: press the trigger, drag onto an item, let go.
    #[gpui::test]
    fn pressing_the_trigger_and_releasing_over_an_item_picks_it(cx: &mut TestAppContext) {
        cx.update(|cx| crate::init(cx));
        let picked = Rc::new(Cell::new(0));
        let (_, cx) = cx.add_window_view({
            let picked = picked.clone();
            move |_, _| PickRoot { picked }
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));

        cx.simulate_mouse_down(
            point(px(10.), px(10.)),
            MouseButton::Left,
            Default::default(),
        );
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let target = cx
            .debug_bounds("pick-target")
            .expect("the press opens the menu")
            .center();
        cx.simulate_mouse_move(target, MouseButton::Left, Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_mouse_up(target, MouseButton::Left, Default::default());
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));

        assert_eq!(picked.get(), 1);
        assert!(
            cx.debug_bounds("pick-target").is_none(),
            "and the menu closes"
        );
    }

    /// A quick click opens the menu and leaves it open: its release over the
    /// trigger picks nothing.
    #[gpui::test]
    fn a_quick_click_on_the_trigger_leaves_the_menu_open(cx: &mut TestAppContext) {
        cx.update(|cx| crate::init(cx));
        let picked = Rc::new(Cell::new(0));
        let (_, cx) = cx.add_window_view({
            let picked = picked.clone();
            move |_, _| PickRoot { picked }
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));

        cx.simulate_click(point(px(10.), px(10.)), Default::default());
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));

        assert_eq!(picked.get(), 0);
        assert!(cx.debug_bounds("pick-target").is_some());
    }

    /// A menu with an option bound to the root's state, recording the menu.
    struct OptionRoot {
        wrap: bool,
        menu: Rc<RefCell<Option<WeakEntity<PopupMenu>>>>,
    }

    impl Render for OptionRoot {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let wrap = self.wrap;
            let slot = self.menu.clone();
            div()
                .size_full()
                .on_action(cx.listener(|this, _: &ToggleWrap, _, cx| {
                    this.wrap = !this.wrap;
                    cx.notify();
                }))
                .child(
                    Button::new("trigger")
                        .label("View")
                        .w(px(100.))
                        .h(px(30.))
                        .dropdown_menu(move |menu, _, cx| {
                            *slot.borrow_mut() = Some(cx.weak_entity());
                            menu.menu_with_check("Wrap lines", wrap, Box::new(ToggleWrap))
                                .menu("Copy", Box::new(CopyText))
                        }),
                )
        }
    }

    #[gpui::test]
    fn a_chosen_option_shows_its_new_state_while_the_menu_stays_open(cx: &mut TestAppContext) {
        cx.update(|cx| crate::init(cx));
        let menu = Rc::new(RefCell::new(None::<WeakEntity<PopupMenu>>));
        let (_, cx) = cx.add_window_view({
            let menu = menu.clone();
            move |_, _| OptionRoot { wrap: false, menu }
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_click(point(px(10.), px(10.)), Default::default());
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));

        cx.simulate_keystrokes("down enter");
        cx.update(|window, cx| window.draw(cx).clear(cx));

        let menu = menu
            .borrow()
            .as_ref()
            .and_then(|menu| menu.upgrade())
            .expect("the menu stays open");
        cx.update(|_, cx| {
            assert!(
                gpui_base::GlobalState::is_in_deferred_context(cx),
                "the menu stays open"
            );
        });
        menu.read_with(cx, |menu, _| {
            assert!(menu.is_item_checked(0), "the option shows its new state");
            assert_eq!(menu.highlighted(), Some(0), "and keeps its highlight");
        });
    }
}
