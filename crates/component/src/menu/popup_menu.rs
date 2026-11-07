use crate::ThemeStyled as _;
use crate::actions::{Cancel, Confirm, SelectDown, SelectFirst, SelectLast, SelectUp};
use crate::actions::{SelectLeft, SelectRight};
use crate::menu::menu_item::MenuItemElement;
use crate::scroll::ScrollableElement;
use crate::{ActiveTheme, ElementExt, Icon, IconName, Sizable as _, h_flex, v_flex};
use crate::{Side, Size, kbd::Kbd};
use gpui::{
    Action, Anchor, AnyElement, App, AppContext, Bounds, Context, DismissEvent, DispatchPhase,
    Edges, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyBinding, KeyDownEvent, MouseButton, MouseClickEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, Pixels, Rems, Render, Role, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Task, WeakEntity, Window, anchored, canvas, deferred, div,
    hover_intent, prelude::FluentBuilder, px, rems,
};
use gpui::{ClickEvent, Half, MouseDownEvent, OwnedMenuItem, Point, Subscription};
use gpui_base::TestSupportExt as _;

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

const CONTEXT: &str = "PopupMenu";

/// How many frames a newly opened scrollable menu waits for its items to be
/// measured before it gives up scrolling to the checked item.
const SCROLL_TO_CHECKED_FRAMES: u8 = 4;

/// How long the pointer rests on a submenu's item before the submenu opens,
/// Base UI's `delay` for a submenu trigger. It closes with no delay.
const SUBMENU_OPEN_DELAY: Duration = Duration::from_millis(100);

/// How long the pointer may rest on its way to an open submenu before the
/// item under it takes the highlight, so a pause on the way never sticks.
const SUBMENU_REST_DELAY: Duration = Duration::from_millis(100);

/// How long typed characters are remembered for typeahead, Base UI's figure.
const TYPEAHEAD_RESET_DELAY: Duration = Duration::from_millis(500);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", Confirm { secondary: false }, Some(CONTEXT)),
        KeyBinding::new("escape", Cancel, Some(CONTEXT)),
        KeyBinding::new("up", SelectUp, Some(CONTEXT)),
        KeyBinding::new("down", SelectDown, Some(CONTEXT)),
        KeyBinding::new("left", SelectLeft, Some(CONTEXT)),
        KeyBinding::new("right", SelectRight, Some(CONTEXT)),
        KeyBinding::new("home", SelectFirst, Some(CONTEXT)),
        KeyBinding::new("end", SelectLast, Some(CONTEXT)),
    ]);
    super::dropdown_menu::init(cx);
}

/// Which end of a menu a highlight goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MenuEdge {
    First,
    Last,
}

/// One menu's highlight, kept across a rebuild of its items.
struct MenuLevel {
    selected: Option<usize>,
    submenu: Option<usize>,
    focused: bool,
}

/// An menu item in a popup menu.
pub enum PopupMenuItem {
    /// A menu separator item.
    Separator,
    /// A non-interactive label item.
    Label(SharedString),
    /// A standard menu item.
    Item {
        icon: Option<Icon>,
        label: SharedString,
        disabled: bool,
        checked: bool,
        is_link: bool,
        action: Option<Box<dyn Action>>,
        // For link item
        handler: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
    },
    /// A menu item with custom element render.
    ElementItem {
        icon: Option<Icon>,
        disabled: bool,
        checked: bool,
        action: Option<Box<dyn Action>>,
        render: Box<dyn Fn(&mut Window, &mut App) -> AnyElement + 'static>,
        handler: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
    },
    /// A submenu item that opens another popup menu.
    Submenu {
        icon: Option<Icon>,
        label: SharedString,
        disabled: bool,
        menu: Entity<PopupMenu>,
    },
}

impl FluentBuilder for PopupMenuItem {}
impl PopupMenuItem {
    /// Create a new menu item with the given label.
    #[inline]
    pub fn new(label: impl Into<SharedString>) -> Self {
        PopupMenuItem::Item {
            icon: None,
            label: label.into(),
            disabled: false,
            checked: false,
            action: None,
            is_link: false,
            handler: None,
        }
    }

    /// Create a new menu item with custom element render.
    #[inline]
    pub fn element<F, E>(builder: F) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        PopupMenuItem::ElementItem {
            icon: None,
            disabled: false,
            checked: false,
            action: None,
            render: Box::new(move |window, cx| builder(window, cx).into_any_element()),
            handler: None,
        }
    }

    /// Create a new submenu item that opens another popup menu.
    #[inline]
    pub fn submenu(label: impl Into<SharedString>, menu: Entity<PopupMenu>) -> Self {
        PopupMenuItem::Submenu {
            icon: None,
            label: label.into(),
            disabled: false,
            menu,
        }
    }

    /// Create a separator menu item.
    #[inline]
    pub fn separator() -> Self {
        PopupMenuItem::Separator
    }

    /// Creates a label menu item.
    #[inline]
    pub fn label(label: impl Into<SharedString>) -> Self {
        PopupMenuItem::Label(label.into())
    }

    /// Set the icon for the menu item.
    ///
    /// Only works for [`PopupMenuItem::Item`], [`PopupMenuItem::ElementItem`] and [`PopupMenuItem::Submenu`].
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        match &mut self {
            PopupMenuItem::Item { icon: i, .. } => {
                *i = Some(icon.into());
            }
            PopupMenuItem::ElementItem { icon: i, .. } => {
                *i = Some(icon.into());
            }
            PopupMenuItem::Submenu { icon: i, .. } => {
                *i = Some(icon.into());
            }
            _ => {}
        }
        self
    }

    /// Set the action for the menu item.
    ///
    /// Only works for [`PopupMenuItem::Item`] and [`PopupMenuItem::ElementItem`].
    pub fn action(mut self, action: Box<dyn Action>) -> Self {
        match &mut self {
            PopupMenuItem::Item { action: a, .. } => {
                *a = Some(action);
            }
            PopupMenuItem::ElementItem { action: a, .. } => {
                *a = Some(action);
            }
            _ => {}
        }
        self
    }

    /// Set the disabled state for the menu item.
    ///
    /// Only works for [`PopupMenuItem::Item`], [`PopupMenuItem::ElementItem`] and [`PopupMenuItem::Submenu`].
    pub fn disabled(mut self, disabled: bool) -> Self {
        match &mut self {
            PopupMenuItem::Item { disabled: d, .. } => {
                *d = disabled;
            }
            PopupMenuItem::ElementItem { disabled: d, .. } => {
                *d = disabled;
            }
            PopupMenuItem::Submenu { disabled: d, .. } => {
                *d = disabled;
            }
            _ => {}
        }
        self
    }

    /// Set checked state for the menu item.
    ///
    /// NOTE: If `check_side` is [`Side::Left`], the icon will replace with a check icon.
    pub fn checked(mut self, checked: bool) -> Self {
        match &mut self {
            PopupMenuItem::Item { checked: c, .. } => {
                *c = checked;
            }
            PopupMenuItem::ElementItem { checked: c, .. } => {
                *c = checked;
            }
            _ => {}
        }
        self
    }

    /// Add a click handler for the menu item.
    ///
    /// Only works for [`PopupMenuItem::Item`] and [`PopupMenuItem::ElementItem`].
    pub fn on_click<F>(mut self, handler: F) -> Self
    where
        F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    {
        match &mut self {
            PopupMenuItem::Item { handler: h, .. } => {
                *h = Some(Rc::new(handler));
            }
            PopupMenuItem::ElementItem { handler: h, .. } => {
                *h = Some(Rc::new(handler));
            }
            _ => {}
        }
        self
    }

    /// Create a link menu item.
    #[inline]
    pub fn link(label: impl Into<SharedString>, href: impl Into<String>) -> Self {
        let href = href.into();
        PopupMenuItem::Item {
            icon: None,
            label: label.into(),
            disabled: false,
            checked: false,
            action: None,
            is_link: true,
            handler: Some(Rc::new(move |_, _, cx| cx.open_url(&href))),
        }
    }

    #[inline]
    fn is_clickable(&self) -> bool {
        !matches!(self, PopupMenuItem::Separator)
            && matches!(
                self,
                PopupMenuItem::Item {
                    disabled: false,
                    ..
                } | PopupMenuItem::ElementItem {
                    disabled: false,
                    ..
                } | PopupMenuItem::Submenu {
                    disabled: false,
                    ..
                }
            )
    }

    #[inline]
    fn is_separator(&self) -> bool {
        matches!(self, PopupMenuItem::Separator)
    }

    #[inline]
    fn is_submenu(&self) -> bool {
        matches!(self, PopupMenuItem::Submenu { .. })
    }

    fn has_left_icon(&self, check_side: Side) -> bool {
        match self {
            PopupMenuItem::Item { icon, checked, .. } => {
                icon.is_some() || (check_side.is_left() && *checked)
            }
            PopupMenuItem::ElementItem { icon, checked, .. } => {
                icon.is_some() || (check_side.is_left() && *checked)
            }
            PopupMenuItem::Submenu { icon, .. } => icon.is_some(),
            _ => false,
        }
    }

    #[inline]
    fn is_checked(&self) -> bool {
        match self {
            PopupMenuItem::Item { checked, .. } => *checked,
            PopupMenuItem::ElementItem { checked, .. } => *checked,
            _ => false,
        }
    }

    fn a11y_label(&self) -> Option<SharedString> {
        match self {
            PopupMenuItem::Item { label, .. }
            | PopupMenuItem::Label(label)
            | PopupMenuItem::Submenu { label, .. } => Some(label.clone()),
            PopupMenuItem::Separator | PopupMenuItem::ElementItem { .. } => None,
        }
    }
}

pub struct PopupMenu {
    pub(crate) focus_handle: FocusHandle,
    pub(crate) menu_items: Vec<PopupMenuItem>,
    /// The focus handle of Entity to handle actions.
    pub(crate) action_context: Option<FocusHandle>,
    /// The focus to restore on dismiss. Unlike `action_context`, this does not
    /// change where actions are dispatched: they still bubble from the menu's
    /// own focus path (through the trigger element's ancestors).
    pub(crate) previous_focus_handle: Option<FocusHandle>,
    /// A focus handle on the trigger's dispatch path, so shortcut hints can
    /// resolve against the key contexts the menu's actions bubble through on
    /// the very frame the menu opens. GPUI looks a handle up in the previously
    /// rendered frame, and the trigger was in it when the menu was not yet.
    pub(crate) trigger_focus_handle: Option<FocusHandle>,
    /// The highlighted item, by keyboard or pointer.
    selected_index: Option<usize>,
    /// The item under the pointer, which may differ from the highlighted one
    /// while the pointer travels to an open submenu.
    hovered_index: Option<usize>,
    /// The item whose submenu is open.
    submenu_index: Option<usize>,
    /// Opens a hovered submenu item's submenu once the pointer has rested on
    /// it for [`SUBMENU_OPEN_DELAY`].
    submenu_open_task: Option<Task<()>>,
    /// Whether the pointer was over the open submenu's item when last seen.
    on_submenu_item: bool,
    /// Where the pointer left the open submenu's item, while it heads for the
    /// submenu (see [`hover_intent::heading_to`]). Siblings it crosses on the
    /// way do not take the highlight.
    submenu_apex: Option<Point<Pixels>>,
    /// Hands the highlight to the item under a pointer that rested on its way
    /// to the submenu for [`SUBMENU_REST_DELAY`].
    submenu_rest_task: Option<Task<()>>,
    /// Each item's bounds as last laid out, by index.
    item_bounds: Rc<RefCell<Vec<Bounds<Pixels>>>>,
    /// Characters typed for typeahead, lowercased, until
    /// [`TYPEAHEAD_RESET_DELAY`] passes without typing.
    typeahead: String,
    typeahead_task: Option<Task<()>>,
    /// The mouse button still held from the press that opened the menu:
    /// releasing it over an item activates the item, as a native menu does.
    release_button: Option<MouseButton>,
    /// An option was activated and the menu stayed open, so its items no
    /// longer show the current state. Set on the top-level menu, which its
    /// owner rebuilds with [`PopupMenu::rebuild_stale`].
    stale: bool,
    min_width: Option<Pixels>,
    max_width: Option<Pixels>,
    max_height: Option<Pixels>,
    bounds: Bounds<Pixels>,
    size: Size,
    check_side: Side,
    /// Indexes of the items added as options that can be checked, so an
    /// unchecked one reports its state rather than reading as a command.
    checkable: HashSet<usize>,

    /// The parent menu of this menu, if this is a submenu
    parent_menu: Option<WeakEntity<Self>>,
    scrollable: bool,
    /// Frames left to scroll the checked item into view on open. Zero once
    /// done, so the user's scrolling is not overridden afterwards.
    scroll_to_checked_frames: u8,
    external_link_icon: bool,
    scroll_handle: ScrollHandle,
    // This will update on render
    submenu_anchor: (Anchor, Pixels),

    /// Paint priority for this menu layer. The top-level menu starts at 1 and
    /// each nested submenu increments it, so deeper levels are always drawn on
    /// top of shallower ones. This fixes background content (e.g. the
    /// underlying list) bleeding through multi-level submenus, which happens
    /// when nested `anchored` popovers share the same paint order.
    ///
    /// The top-level menu relies on its container (e.g. `Popover`,
    /// `ContextMenu`) to `deferred`-draw it, and each submenu is deferred once
    /// in `render_item` with `priority + 1`. Keeping a single deferred layer
    /// per level matters because GPUI caps nested deferred depth (see
    /// `prepaint_deferred_draws`).
    ///
    /// Deferring is also what lets a submenu open from a `scrollable` menu:
    /// GPUI paints a deferred draw at the window level with no inherited
    /// content mask, so the items container's `overflow_y_scroll` clip never
    /// reaches it, while the scroll offset is still baked into its anchor.
    priority: usize,

    _subscriptions: Vec<Subscription>,
}

impl PopupMenu {
    pub(crate) fn new(cx: &mut App) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            action_context: None,
            previous_focus_handle: None,
            trigger_focus_handle: None,
            parent_menu: None,
            menu_items: Vec::new(),
            selected_index: None,
            hovered_index: None,
            submenu_index: None,
            submenu_open_task: None,
            on_submenu_item: false,
            submenu_apex: None,
            submenu_rest_task: None,
            item_bounds: Rc::default(),
            typeahead: String::new(),
            typeahead_task: None,
            release_button: None,
            stale: false,
            min_width: None,
            max_width: None,
            max_height: None,
            check_side: Side::Left,
            checkable: HashSet::new(),
            bounds: Bounds::default(),
            scrollable: false,
            scroll_to_checked_frames: SCROLL_TO_CHECKED_FRAMES,
            scroll_handle: ScrollHandle::default(),
            external_link_icon: true,
            size: Size::default(),
            submenu_anchor: (Anchor::TopLeft, Pixels::ZERO),
            priority: gpui_base::POPUP_PRIORITY,
            _subscriptions: vec![],
        }
    }

    pub fn build(
        window: &mut Window,
        cx: &mut App,
        f: impl FnOnce(Self, &mut Window, &mut Context<PopupMenu>) -> Self,
    ) -> Entity<Self> {
        cx.new(|cx| f(Self::new(cx), window, cx))
    }

    /// Set the focus handle of Entity to handle actions.
    ///
    /// When the menu is dismissed or before an action is triggered, the focus will be returned to this handle.
    ///
    /// Then the action will be dispatched to this handle.
    /// Submenus without their own action context use the nearest parent's
    /// explicit context for actions and shortcut hints.
    pub fn action_context(mut self, handle: FocusHandle) -> Self {
        self.action_context = Some(handle);
        self
    }

    pub(crate) fn set_action_context(
        &mut self,
        action_context: Option<FocusHandle>,
        cx: &mut Context<Self>,
    ) {
        self.action_context = action_context.clone();

        for item in &self.menu_items {
            if let PopupMenuItem::Submenu { menu, .. } = item {
                menu.update(cx, |menu, cx| {
                    menu.set_action_context(action_context.clone(), cx);
                });
            }
        }
    }

    /// Set the focus to restore when the menu is dismissed, without changing
    /// where actions are dispatched.
    pub(crate) fn set_previous_focus(
        &mut self,
        handle: Option<FocusHandle>,
        cx: &mut Context<Self>,
    ) {
        self.previous_focus_handle = handle.clone();

        for item in &self.menu_items {
            if let PopupMenuItem::Submenu { menu, .. } = item {
                menu.update(cx, |menu, cx| {
                    menu.set_previous_focus(handle.clone(), cx);
                });
            }
        }
    }

    /// Set the focus handle on the trigger's dispatch path that shortcut hints
    /// resolve against, without changing focus or where actions are dispatched.
    pub(crate) fn set_trigger_focus(
        &mut self,
        handle: Option<FocusHandle>,
        cx: &mut Context<Self>,
    ) {
        self.trigger_focus_handle = handle.clone();

        for item in &self.menu_items {
            if let PopupMenuItem::Submenu { menu, .. } = item {
                menu.update(cx, |menu, cx| {
                    menu.set_trigger_focus(handle.clone(), cx);
                });
            }
        }
    }

    /// Set min width of the popup menu, default is 8rem
    pub fn min_w(mut self, width: impl Into<Pixels>) -> Self {
        self.min_width = Some(width.into());
        self
    }

    /// Set max width of the popup menu, default is 31.25rem (500px at the
    /// default 16px rem)
    pub fn max_w(mut self, width: impl Into<Pixels>) -> Self {
        self.max_width = Some(width.into());
        self
    }

    /// Set max height of the popup menu, default is half of the window height,
    /// at most 28.125rem (450px at the default 16px rem)
    pub fn max_h(mut self, height: impl Into<Pixels>) -> Self {
        self.max_height = Some(height.into());
        self
    }

    /// Set the menu to be scrollable to show vertical scrollbar.
    pub fn scrollable(mut self, scrollable: bool) -> Self {
        self.scrollable = scrollable;
        self
    }

    /// Set the side to show check icon, default is `Side::Left`.
    pub fn check_side(mut self, side: Side) -> Self {
        self.check_side = side;
        self
    }

    /// Set the menu to show external link icon, default is true.
    pub fn external_link_icon(mut self, visible: bool) -> Self {
        self.external_link_icon = visible;
        self
    }

    /// Add Menu Item
    pub fn menu(self, label: impl Into<SharedString>, action: Box<dyn Action>) -> Self {
        self.menu_with_disabled(label, action, false)
    }

    /// Add Menu Item with enable state
    pub fn menu_with_enable(
        mut self,
        label: impl Into<SharedString>,
        action: Box<dyn Action>,
        enable: bool,
    ) -> Self {
        self.add_menu_item(label, None, action, !enable, false);
        self
    }

    /// Add Menu Item with disabled state
    pub fn menu_with_disabled(
        mut self,
        label: impl Into<SharedString>,
        action: Box<dyn Action>,
        disabled: bool,
    ) -> Self {
        self.add_menu_item(label, None, action, disabled, false);
        self
    }

    /// Add label
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.menu_items.push(PopupMenuItem::label(label.into()));
        self
    }

    /// Add Menu to open link
    pub fn link(self, label: impl Into<SharedString>, href: impl Into<String>) -> Self {
        self.link_with_disabled(label, href, false)
    }

    /// Add Menu to open link with disabled state
    pub fn link_with_disabled(
        mut self,
        label: impl Into<SharedString>,
        href: impl Into<String>,
        disabled: bool,
    ) -> Self {
        let href = href.into();
        self.menu_items
            .push(PopupMenuItem::link(label, href).disabled(disabled));
        self
    }

    /// Add Menu to open link
    pub fn link_with_icon(
        self,
        label: impl Into<SharedString>,
        icon: impl Into<Icon>,
        href: impl Into<String>,
    ) -> Self {
        self.link_with_icon_and_disabled(label, icon, href, false)
    }

    /// Add Menu to open link with icon and disabled state
    fn link_with_icon_and_disabled(
        mut self,
        label: impl Into<SharedString>,
        icon: impl Into<Icon>,
        href: impl Into<String>,
        disabled: bool,
    ) -> Self {
        let href = href.into();
        self.menu_items.push(
            PopupMenuItem::link(label, href)
                .icon(icon)
                .disabled(disabled),
        );
        self
    }

    /// Add Menu Item with Icon.
    pub fn menu_with_icon(
        self,
        label: impl Into<SharedString>,
        icon: impl Into<Icon>,
        action: Box<dyn Action>,
    ) -> Self {
        self.menu_with_icon_and_disabled(label, icon, action, false)
    }

    /// Add Menu Item with Icon and disabled state
    pub fn menu_with_icon_and_disabled(
        mut self,
        label: impl Into<SharedString>,
        icon: impl Into<Icon>,
        action: Box<dyn Action>,
        disabled: bool,
    ) -> Self {
        self.add_menu_item(label, Some(icon.into()), action, disabled, false);
        self
    }

    /// Add Menu Item with check icon
    pub fn menu_with_check(
        self,
        label: impl Into<SharedString>,
        checked: bool,
        action: Box<dyn Action>,
    ) -> Self {
        self.menu_with_check_and_disabled(label, checked, action, false)
    }

    /// Add Menu Item with check icon and disabled state
    pub fn menu_with_check_and_disabled(
        mut self,
        label: impl Into<SharedString>,
        checked: bool,
        action: Box<dyn Action>,
        disabled: bool,
    ) -> Self {
        self.checkable.insert(self.menu_items.len());
        self.add_menu_item(label, None, action, disabled, checked);
        self
    }

    /// Add Menu Item with custom element render.
    pub fn menu_element<F, E>(self, action: Box<dyn Action>, builder: F) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.menu_element_with_check(false, action, builder)
    }

    /// Add Menu Item with custom element render with disabled state.
    pub fn menu_element_with_disabled<F, E>(
        self,
        action: Box<dyn Action>,
        disabled: bool,
        builder: F,
    ) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.menu_element_with_check_and_disabled(false, action, disabled, builder)
    }

    /// Add Menu Item with custom element render with icon.
    pub fn menu_element_with_icon<F, E>(
        self,
        icon: impl Into<Icon>,
        action: Box<dyn Action>,
        builder: F,
    ) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.menu_element_with_icon_and_disabled(icon, action, false, builder)
    }

    /// Add Menu Item with custom element render with check state
    pub fn menu_element_with_check<F, E>(
        self,
        checked: bool,
        action: Box<dyn Action>,
        builder: F,
    ) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.menu_element_with_check_and_disabled(checked, action, false, builder)
    }

    /// Add Menu Item with custom element render with icon and disabled state
    fn menu_element_with_icon_and_disabled<F, E>(
        mut self,
        icon: impl Into<Icon>,
        action: Box<dyn Action>,
        disabled: bool,
        builder: F,
    ) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.menu_items.push(
            PopupMenuItem::element(builder)
                .action(action)
                .icon(icon)
                .disabled(disabled),
        );
        self
    }

    /// Add Menu Item with custom element render with check state and disabled state
    fn menu_element_with_check_and_disabled<F, E>(
        mut self,
        checked: bool,
        action: Box<dyn Action>,
        disabled: bool,
        builder: F,
    ) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.checkable.insert(self.menu_items.len());
        self.menu_items.push(
            PopupMenuItem::element(builder)
                .action(action)
                .checked(checked)
                .disabled(disabled),
        );
        self
    }

    /// Add a separator Menu Item
    pub fn separator(mut self) -> Self {
        if self.menu_items.is_empty() {
            return self;
        }

        if let Some(PopupMenuItem::Separator) = self.menu_items.last() {
            return self;
        }

        self.menu_items.push(PopupMenuItem::separator());
        self
    }

    /// Add a Submenu
    pub fn submenu(
        self,
        label: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        self.submenu_with_icon(None, label, window, cx, f)
    }

    /// Add a Submenu item with icon
    pub fn submenu_with_icon(
        mut self,
        icon: Option<Icon>,
        label: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        let submenu = PopupMenu::build(window, cx, f);
        let parent_menu = cx.entity().downgrade();
        let parent_priority = self.priority;
        submenu.update(cx, |view, _| {
            view.parent_menu = Some(parent_menu);
            view.priority = parent_priority + 1;
        });

        self.menu_items.push(
            PopupMenuItem::submenu(label, submenu).when_some(icon, |this, icon| this.icon(icon)),
        );
        self
    }

    /// Add menu item.
    pub fn item(mut self, item: impl Into<PopupMenuItem>) -> Self {
        let item: PopupMenuItem = item.into();
        self.menu_items.push(item);
        self
    }

    /// Replace all menu items by re-running a builder on this menu, keeping its
    /// identity (focus, parent menu, layer priority).
    ///
    /// For menus whose content arrives asynchronously after the menu is shown,
    /// e.g. swapping a "loading…" placeholder for the loaded items:
    ///
    /// ```ignore
    /// cx.spawn_in(window, async move |menu, cx| {
    ///     let items = fetch_items().await;
    ///     _ = menu.update_in(cx, |menu, window, cx| {
    ///         menu.rebuild(window, cx, |menu, _, _| {
    ///             items.into_iter().fold(menu, |menu, item| {
    ///                 menu.menu(item.label, Box::new(item.action))
    ///             })
    ///         });
    ///     });
    /// })
    /// .detach();
    /// ```
    pub fn rebuild(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(Self, &mut Window, &mut Context<Self>) -> Self,
    ) {
        let mut menu = std::mem::replace(self, Self::new(cx));
        menu.menu_items.clear();
        menu.checkable.clear();
        menu.selected_index = None;
        menu.hovered_index = None;
        menu.forget_submenu();
        *self = f(menu, window, cx);
        cx.notify();
    }

    /// Whether an option was activated with the menu left open, so its items
    /// must be built again to show the current state.
    pub(crate) fn is_stale(&self) -> bool {
        self.stale
    }

    /// Rebuild a stale menu from its owner's current builder, keeping the
    /// highlight, the open submenus and the focus where they were.
    pub(crate) fn rebuild_stale(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(Self, &mut Window, &mut Context<Self>) -> Self,
    ) {
        if !self.stale {
            return;
        }
        self.stale = false;
        let path = self.highlight_path(window, cx);
        self.rebuild(window, cx, f);
        // Submenus are built anew, so they learn the focus to restore and the
        // trigger's key contexts again.
        self.set_previous_focus(self.previous_focus_handle.clone(), cx);
        self.set_trigger_focus(self.trigger_focus_handle.clone(), cx);
        self.restore_highlight_path(&path, window, cx);
    }

    fn highlight_path(&self, window: &Window, cx: &App) -> Vec<MenuLevel> {
        let mut path = vec![MenuLevel {
            selected: self.selected_index,
            submenu: self.submenu_index,
            focused: self.focus_handle.is_focused(window),
        }];
        if let Some(submenu) = self.active_submenu() {
            path.extend(submenu.read(cx).highlight_path(window, cx));
        }
        path
    }

    fn restore_highlight_path(
        &mut self,
        path: &[MenuLevel],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((level, rest)) = path.split_first() else {
            return;
        };
        self.selected_index = level.selected.filter(|ix| {
            self.menu_items
                .get(*ix)
                .is_some_and(PopupMenuItem::is_clickable)
        });
        self.submenu_index = level.submenu.filter(|ix| {
            self.menu_items
                .get(*ix)
                .is_some_and(|item| item.is_submenu() && item.is_clickable())
        });
        if level.focused {
            self.focus_handle.focus(window, cx);
        }
        if let Some(submenu) = self.active_submenu() {
            submenu.update(cx, |submenu, cx| {
                submenu.restore_highlight_path(rest, window, cx)
            });
        }
    }

    /// Mark the top-level menu stale, for its owner to rebuild.
    fn mark_stale(&mut self, cx: &mut Context<Self>) {
        match self
            .parent_menu
            .as_ref()
            .and_then(|parent| parent.upgrade())
        {
            Some(parent) => parent.update(cx, |parent, cx| parent.mark_stale(cx)),
            None => {
                self.stale = true;
                cx.notify();
            }
        }
    }

    /// Arm the press that opened the menu: releasing `button` over an item
    /// activates it, so a press on the trigger can drag to an item and let go.
    /// Any later press disarms it.
    pub(crate) fn arm_release(&mut self, button: MouseButton) {
        self.release_button = Some(button);
    }

    /// Highlight the first or last enabled item.
    pub(crate) fn highlight_edge(&mut self, edge: MenuEdge, cx: &mut Context<Self>) {
        let ix = match edge {
            MenuEdge::First => self.clickable_menu_items().next(),
            MenuEdge::Last => self.clickable_menu_items().last(),
        }
        .map(|(ix, _)| ix);
        if let Some(ix) = ix {
            self.highlight(ix, cx);
        }
    }

    fn add_menu_item(
        &mut self,
        label: impl Into<SharedString>,
        icon: Option<Icon>,
        action: Box<dyn Action>,
        disabled: bool,
        checked: bool,
    ) -> &mut Self {
        self.menu_items.push(
            PopupMenuItem::new(label)
                .when_some(icon, |item, icon| item.icon(icon))
                .disabled(disabled)
                .checked(checked)
                .action(action),
        );
        self
    }

    pub(super) fn with_menu_items<I>(
        mut self,
        items: impl IntoIterator<Item = I>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self
    where
        I: Into<OwnedMenuItem>,
    {
        for item in items {
            match item.into() {
                OwnedMenuItem::Action {
                    name,
                    action,
                    checked,
                    disabled,
                    ..
                } => {
                    // Every application menu action has a check state, so an
                    // unchecked one is not known to be an option.
                    self = self.item(
                        PopupMenuItem::new(name)
                            .action(action.boxed_clone())
                            .disabled(disabled)
                            .checked(checked),
                    )
                }
                OwnedMenuItem::Separator => {
                    self = self.separator();
                }
                OwnedMenuItem::Submenu(submenu) => {
                    self = self.submenu(submenu.name, window, cx, move |menu, window, cx| {
                        menu.with_menu_items(submenu.items.clone(), window, cx)
                    })
                }
                OwnedMenuItem::SystemMenu(_) => {}
            }
        }

        if self.menu_items.len() > 20 {
            self.scrollable = true;
        }

        self
    }

    /// The open submenu.
    pub(crate) fn active_submenu(&self) -> Option<Entity<PopupMenu>> {
        match self.menu_items.get(self.submenu_index?) {
            Some(PopupMenuItem::Submenu { menu, .. }) => Some(menu.clone()),
            _ => None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.menu_items.is_empty()
    }

    fn clickable_menu_items(&self) -> impl Iterator<Item = (usize, &PopupMenuItem)> {
        self.menu_items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.is_clickable())
    }

    fn on_click(
        &mut self,
        ix: usize,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        window.prevent_default();
        self.selected_index = Some(ix);
        self.activate(event, window, cx);
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        self.activate(&ClickEvent::default(), window, cx);
    }

    fn activate(&mut self, event: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.selected_index else {
            return;
        };
        let Some(item) = self
            .menu_items
            .get(index)
            .filter(|item| item.is_clickable())
        else {
            return;
        };
        let (handler, action) = match item {
            PopupMenuItem::Item {
                handler, action, ..
            }
            | PopupMenuItem::ElementItem {
                handler, action, ..
            } => (
                handler.clone(),
                action.as_ref().map(|action| action.boxed_clone()),
            ),
            PopupMenuItem::Submenu { .. } => {
                self.open_selected_submenu(window, cx);
                return;
            }
            PopupMenuItem::Separator | PopupMenuItem::Label(_) => return,
        };

        let mut dispatched = false;
        if let Some(handler) = handler {
            handler(event, window, cx);
        } else if let Some(action) = action {
            self.dispatch_confirm_action(&action, window, cx);
            dispatched = true;
        }

        if self.checkable.contains(&index) {
            // An option, like Base UI's checkbox and radio items, leaves the
            // menu open so several can be set in one visit. Its action was
            // dispatched from the action context, so the menu takes the focus
            // back, and its owner rebuilds the items to show the new state.
            if dispatched {
                self.focus_handle.focus(window, cx);
            }
            self.mark_stale(cx);
        } else {
            self.dismiss(&Cancel, window, cx)
        }
    }

    fn dispatch_confirm_action(
        &self,
        action: &Box<dyn Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(context) = self.resolved_action_context(cx) {
            context.focus(window, cx);
        }

        window.dispatch_action(action.boxed_clone(), cx);
    }

    fn resolved_action_context(&self, cx: &App) -> Option<FocusHandle> {
        self.action_context.clone().or_else(|| {
            let parent = self.parent_menu.as_ref()?.upgrade()?;
            parent.read(cx).resolved_action_context(cx)
        })
    }

    fn set_selected_index(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.selected_index != Some(ix) {
            self.selected_index = Some(ix);
            self.scroll_handle.scroll_to_item(ix);
            cx.notify();
        }
    }

    /// Scroll a scrollable menu once, as it opens, so its checked item is the
    /// first one in view, the way a native picker opens on its current value.
    ///
    /// `ScrollHandle::scroll_to_top_of_item` is not used: it measures the item
    /// against the container bounds recorded on the previous frame, which a
    /// just-opened `anchored` menu does not have at its final position yet, so
    /// it would land off by the distance the menu moved. The offset here is
    /// the distance between the first item and the checked one, measured in
    /// the same frame, so it does not depend on where the container is. Items
    /// are measured on the first prepaint, so until then this asks for another
    /// frame, up to `SCROLL_TO_CHECKED_FRAMES` times.
    fn scroll_to_checked_item(&mut self, cx: &mut Context<Self>) {
        if !self.scrollable || self.scroll_to_checked_frames == 0 {
            return;
        }

        // Only a trailing separator is not rendered, so an item's index is
        // also its index among the scroll container's children.
        let checked_ix = self.menu_items.iter().position(|item| item.is_checked());
        let Some(ix) = checked_ix.filter(|ix| *ix > 0) else {
            self.scroll_to_checked_frames = 0;
            return;
        };

        match (
            self.scroll_handle.bounds_for_item(0),
            self.scroll_handle.bounds_for_item(ix),
        ) {
            (Some(first), Some(checked)) => {
                self.scroll_to_checked_frames = 0;
                let mut offset = self.scroll_handle.offset();
                offset.y = first.top() - checked.top();
                self.scroll_handle.set_offset(offset);
            }
            _ => {
                self.scroll_to_checked_frames -= 1;
                cx.notify();
            }
        }
    }

    /// Highlight item `ix` from the keyboard, closing a submenu opened from
    /// another item.
    fn highlight(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.submenu_index != Some(ix) {
            self.close_submenu(cx);
        }
        self.set_selected_index(ix, cx);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let ix = self.selected_index.unwrap_or(0);

        if let Some((prev_ix, _)) = self
            .menu_items
            .iter()
            .enumerate()
            .rev()
            .find(|(i, item)| *i < ix && item.is_clickable())
        {
            self.highlight(prev_ix, cx);
            return;
        }

        self.highlight_edge(MenuEdge::Last, cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let Some(ix) = self.selected_index else {
            self.highlight_edge(MenuEdge::First, cx);
            return;
        };

        if let Some((next_ix, _)) = self
            .menu_items
            .iter()
            .enumerate()
            .find(|(i, item)| *i > ix && item.is_clickable())
        {
            self.highlight(next_ix, cx);
            return;
        }

        self.highlight_edge(MenuEdge::First, cx);
    }

    fn select_first(&mut self, _: &SelectFirst, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.highlight_edge(MenuEdge::First, cx);
    }

    fn select_last(&mut self, _: &SelectLast, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.highlight_edge(MenuEdge::Last, cx);
    }

    /// Typeahead: printable characters highlight the next enabled item whose
    /// label starts with what was typed in the last
    /// [`TYPEAHEAD_RESET_DELAY`]; one character typed again and again cycles
    /// through the items starting with it.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Only the focused menu itself: not a submenu's keys bubbling up, nor
        // a control inside an item that takes text.
        if !self.focus_handle.is_focused(window) {
            return;
        }
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
            return;
        }
        let Some(typed) = keystroke.key_char.as_deref() else {
            return;
        };
        if typed.is_empty()
            || typed.chars().any(char::is_control)
            || (typed == " " && self.typeahead.is_empty())
        {
            return;
        }

        cx.stop_propagation();
        self.typeahead.push_str(&typed.to_lowercase());
        self.typeahead_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(TYPEAHEAD_RESET_DELAY).await;
            _ = this.update(cx, |menu, _| menu.typeahead.clear());
        }));
        if let Some(ix) = self.typeahead_match() {
            self.highlight(ix, cx);
        }
    }

    fn typeahead_match(&self) -> Option<usize> {
        let mut chars = self.typeahead.chars();
        let first = chars.next()?;
        let (query, start) = if chars.all(|c| c == first) {
            // The same character again moves on to the next item with it.
            (
                first.to_string(),
                self.selected_index.map_or(0, |ix| ix + 1),
            )
        } else {
            // A longer prefix may still be the highlighted item's.
            (self.typeahead.clone(), self.selected_index.unwrap_or(0))
        };
        let count = self.menu_items.len();
        (0..count)
            .map(|offset| (start + offset) % count)
            .find(|ix| {
                let item = &self.menu_items[*ix];
                item.is_clickable()
                    && item
                        .a11y_label()
                        .is_some_and(|label| label.to_lowercase().starts_with(&query))
            })
    }

    fn select_left(&mut self, _: &SelectLeft, window: &mut Window, cx: &mut Context<Self>) {
        let handled = if matches!(self.submenu_anchor.0, Anchor::TopLeft | Anchor::BottomLeft) {
            self.close_open_submenu(cx)
        } else {
            self.open_selected_submenu(window, cx)
        };

        if self.parent_side(cx).is_left() {
            self._focus_parent_menu(window, cx);
        }

        if handled {
            return;
        }

        // For parent AppMenuBar to handle.
        if self.parent_menu.is_none() {
            cx.propagate();
        }
    }

    fn select_right(&mut self, _: &SelectRight, window: &mut Window, cx: &mut Context<Self>) {
        let handled = if matches!(self.submenu_anchor.0, Anchor::TopLeft | Anchor::BottomLeft) {
            self.open_selected_submenu(window, cx)
        } else {
            self.close_open_submenu(cx)
        };

        if self.parent_side(cx).is_right() {
            self._focus_parent_menu(window, cx);
        }

        if handled {
            return;
        }

        // For parent AppMenuBar to handle.
        if self.parent_menu.is_none() {
            cx.propagate();
        }
    }

    /// Open the highlighted item's submenu at once and move the focus into it,
    /// with its first item highlighted.
    fn open_selected_submenu(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(ix) = self.selected_index else {
            return false;
        };
        let Some(submenu) = self
            .menu_items
            .get(ix)
            .filter(|item| item.is_clickable())
            .and_then(|item| match item {
                PopupMenuItem::Submenu { menu, .. } => Some(menu.clone()),
                _ => None,
            })
        else {
            return false;
        };

        if self.submenu_index != Some(ix) {
            self.close_submenu(cx);
            self.open_submenu_at(ix, cx);
        }
        // Focus the submenu, so that can be handle the action.
        submenu.update(cx, |view, cx| {
            view.highlight_edge(MenuEdge::First, cx);
            view.focus_handle.focus(window, cx);
            cx.notify();
        });
        cx.notify();
        true
    }

    fn open_submenu_at(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.submenu_open_task = None;
        self.submenu_index = Some(ix);
        self.submenu_apex = None;
        self.submenu_rest_task = None;
        self.on_submenu_item = self.hovered_index == Some(ix);
        cx.notify();
    }

    /// Close the open submenu, if any, and say whether there was one.
    fn close_open_submenu(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.submenu_index.is_some();
        self.close_submenu(cx);
        open
    }

    /// Close the open submenu and the ones open from it.
    ///
    /// Not for a submenu to call on its parent: the parent would update the
    /// submenu while it is already being updated. It calls
    /// [`Self::forget_submenu`] on the parent instead.
    fn close_submenu(&mut self, cx: &mut Context<Self>) {
        if let Some(submenu) = self.active_submenu() {
            submenu.update(cx, |submenu, cx| {
                submenu.selected_index = None;
                submenu.hovered_index = None;
                submenu.close_submenu(cx);
            });
            cx.notify();
        }
        self.forget_submenu();
    }

    /// Clear this menu's record of its open submenu, leaving the submenu's
    /// own state alone.
    fn forget_submenu(&mut self) {
        self.submenu_index = None;
        self.submenu_open_task = None;
        self.on_submenu_item = false;
        self.submenu_apex = None;
        self.submenu_rest_task = None;
    }

    fn _focus_parent_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(parent) = self.parent_menu.as_ref() else {
            return;
        };
        let Some(parent) = parent.upgrade() else {
            return;
        };

        // Going back closes this submenu, as Base UI's and native menus do.
        self.selected_index = None;
        self.hovered_index = None;
        self.close_submenu(cx);
        parent.update(cx, |view, cx| {
            view.forget_submenu();
            view.focus_handle.focus(window, cx);
            cx.notify();
        });
    }

    /// The pointer settled on item `ix`: it takes the highlight, closes a
    /// submenu open from another item, and a submenu item opens its own after
    /// [`SUBMENU_OPEN_DELAY`].
    fn point_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.submenu_index != Some(ix) {
            // Pointing elsewhere closes the open submenu; take back the focus
            // the keyboard may have moved into it.
            if self
                .active_submenu()
                .is_some_and(|submenu| submenu.focus_handle(cx).contains_focused(window, cx))
            {
                self.focus_handle.focus(window, cx);
            }
            self.close_submenu(cx);
            if self
                .menu_items
                .get(ix)
                .is_some_and(|item| item.is_submenu() && item.is_clickable())
            {
                self.submenu_open_task = Some(cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(SUBMENU_OPEN_DELAY).await;
                    _ = this.update(cx, |menu, cx| {
                        if menu.selected_index == Some(ix) && menu.hovered_index == Some(ix) {
                            menu.open_submenu_at(ix, cx);
                        }
                    });
                }));
            }
        }
        self.selected_index = Some(ix);
        cx.notify();
    }

    fn on_item_hover(
        &mut self,
        ix: usize,
        hovered: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if hovered {
            self.hovered_index = Some(ix);
            // On its way to the open submenu, the pointer crosses siblings
            // without taking the highlight.
            if self.submenu_apex.is_none() {
                self.point_at(ix, window, cx);
            }
        } else {
            if self.hovered_index == Some(ix) {
                self.hovered_index = None;
            }
            // The open submenu's item keeps its highlight; any other loses
            // it, and a submenu item waiting to open no longer does.
            if self.submenu_index != Some(ix)
                && self.selected_index == Some(ix)
                // A key press ends hover under a still pointer; keep the
                // highlight so the next arrow key moves on from this item.
                && !window.last_input_was_keyboard()
            {
                self.selected_index = None;
                self.submenu_open_task = None;
            }
        }
        cx.notify();
    }

    /// The pointer moved while a submenu is open. Once it leaves the
    /// submenu's item, it is judged on every move: while it heads for the
    /// submenu, the submenu stays and siblings wait; once it turns away, the
    /// item under it takes over. Resting on the way hands over after
    /// [`SUBMENU_REST_DELAY`].
    fn on_travel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(ix), Some(submenu)) = (self.submenu_index, self.active_submenu()) else {
            return;
        };
        let pointer = window.mouse_position();
        let on_item = self
            .item_bounds
            .borrow()
            .get(ix)
            .is_some_and(|bounds| bounds.contains(&pointer));
        if on_item {
            self.on_submenu_item = true;
            self.submenu_apex = None;
            self.submenu_rest_task = None;
            return;
        }
        if std::mem::take(&mut self.on_submenu_item) {
            self.submenu_apex = Some(pointer);
        }
        let Some(apex) = self.submenu_apex else {
            return;
        };

        let popup = submenu.read(cx).bounds;
        if popup.contains(&pointer) {
            // Arrived.
            self.submenu_apex = None;
            self.submenu_rest_task = None;
        } else if hover_intent::heading_to(apex, popup, pointer) {
            self.submenu_rest_task = Some(cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(SUBMENU_REST_DELAY).await;
                _ = this.update_in(cx, |menu, window, cx| {
                    if menu.submenu_apex.is_some() {
                        menu.end_travel(window, cx);
                    }
                });
            }));
        } else {
            self.submenu_rest_task = None;
            self.end_travel(window, cx);
        }
    }

    /// The pointer is no longer on its way to the submenu: the item under it,
    /// if any, takes over.
    fn end_travel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.submenu_apex = None;
        if let Some(ix) = self
            .hovered_index
            .filter(|ix| Some(*ix) != self.submenu_index)
        {
            self.point_at(ix, window, cx);
        }
    }

    /// Activate the item under the pointer as the press that opened the menu
    /// is released over it.
    fn release_on_hovered(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.hovered_index else {
            return;
        };
        // A submenu's item already opens it under the pointer.
        if !self
            .menu_items
            .get(ix)
            .is_some_and(|item| item.is_clickable() && !item.is_submenu())
        {
            return;
        }
        let click = ClickEvent::Mouse(MouseClickEvent {
            down: MouseDownEvent {
                button: event.button,
                position: event.position,
                modifiers: event.modifiers,
                click_count: 1,
                first_mouse: false,
            },
            up: event.clone(),
        });
        self.selected_index = Some(ix);
        self.activate(&click, window, cx);
    }

    /// Follows the pointer for the open submenu and the press that opened
    /// the menu, in window listeners painted over this menu.
    fn render_pointer_watcher(&self, cx: &Context<Self>) -> impl IntoElement {
        let menu = cx.entity().downgrade();
        let travels = self.submenu_index.is_some();
        let release = self.release_button;
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                if travels {
                    let menu = menu.clone();
                    window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture {
                            menu.update(cx, |menu, cx| menu.on_travel(window, cx)).ok();
                        }
                    });
                }
                if let Some(button) = release {
                    let pressed = menu.clone();
                    window.on_mouse_event(move |_: &MouseDownEvent, phase, _, cx| {
                        if phase == DispatchPhase::Capture {
                            pressed
                                .update(cx, |menu, _| menu.release_button = None)
                                .ok();
                        }
                    });
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture
                            && event.button == button
                            && let Some(menu) = menu.upgrade()
                        {
                            release_press(&menu, event, window, cx);
                        }
                    });
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }

    fn parent_side(&self, cx: &App) -> Side {
        let Some(parent) = self.parent_menu.as_ref() else {
            return Side::Left;
        };

        let Some(parent) = parent.upgrade() else {
            return Side::Left;
        };

        match parent.read(cx).submenu_anchor.0 {
            Anchor::TopLeft | Anchor::BottomLeft => Side::Left,
            Anchor::TopRight | Anchor::BottomRight => Side::Right,
            // Center anchors are not used for submenu positioning, but we must cover them.
            _ => Side::Left,
        }
    }

    /// Dismiss the menu and the entire parent chain.
    ///
    /// The submenu is closed together with its parent, same as macOS menus.
    fn dismiss(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_index = None;
        self.hovered_index = None;
        self.release_button = None;
        self.typeahead.clear();
        self.typeahead_task = None;
        // Not `close_submenu`: the submenu may be the one dismissing, and is
        // already being updated.
        self.forget_submenu();
        cx.emit(DismissEvent);

        // Focus back to the previous focused handle, unless the item's click
        // handler has already moved focus elsewhere (e.g. opened a dialog and
        // focused its input) -- stealing focus back would break that.
        let focus_moved_away =
            window.focused(cx).is_some() && !self.focus_handle.contains_focused(window, cx);
        if !focus_moved_away {
            if let Some(handle) = self
                .previous_focus_handle
                .as_ref()
                .or(self.action_context.as_ref())
            {
                window.focus(handle, cx);
            }
        }

        let Some(parent_menu) = self.parent_menu.clone() else {
            return;
        };

        // Dismiss parent menu, when this menu is dismissed
        _ = parent_menu.update(cx, |view, cx| {
            view.dismiss(&Cancel, window, cx);
        });
    }

    fn handle_dismiss(
        &mut self,
        position: &Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Do not dismiss, if click inside the parent menu
        if let Some(parent) = self.parent_menu.as_ref() {
            if let Some(parent) = parent.upgrade() {
                if parent.read(cx).bounds.contains(position) {
                    return;
                }
            }
        }

        // Do not dismiss, if there have an active submenu, the click may be
        // inside the submenu, let the submenu to handle it.
        //
        // Otherwise the submenu will be dismissed before its item's `on_click`.
        if self.active_submenu().is_some() {
            return;
        }

        self.dismiss(&Cancel, window, cx);
    }

    fn on_mouse_down_out(
        &mut self,
        e: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_dismiss(&e.position, window, cx);
    }

    fn render_key_binding(
        &self,
        action: Option<Box<dyn Action>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Kbd> {
        let action = action?;

        // Each handle names a dispatch path GPUI can resolve in the previously
        // rendered frame: the explicit action target, the trigger the menu
        // opened from (already rendered when the menu first draws), the focus
        // the menu interrupted, then the menu's own path once it has been
        // rendered. A binding registered without a key context applies on
        // every path, so it is the last resort rather than the window's
        // leftover context stack, which only holds the path of whatever
        // element happened to paint last.
        let action_context = self.resolved_action_context(cx);
        [
            action_context.as_ref(),
            self.trigger_focus_handle.as_ref(),
            self.previous_focus_handle.as_ref(),
            Some(&self.focus_handle),
        ]
        .into_iter()
        .flatten()
        .find_map(|handle| Kbd::binding_for_action_in(action.as_ref(), handle, window))
        .or_else(|| Kbd::global_binding_for_action(action.as_ref(), window))
        .map(|this| {
            this.p_0()
                .flex_nowrap()
                .border_0()
                .bg(gpui::transparent_white())
        })
    }

    fn render_icon(
        has_icon: bool,
        checked: bool,
        icon: Option<Icon>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        if !has_icon {
            return None;
        }

        let icon = if let Some(icon) = icon {
            icon.clone()
        } else if checked {
            Icon::new(IconName::Check)
        } else {
            Icon::empty()
        };

        Some(icon.xsmall())
    }

    /// Calculate the anchor corner and left offset for child submenu
    fn update_submenu_menu_anchor(&mut self, window: &Window, cx: &App) {
        let bounds = self.bounds;
        // Open the submenu on the left only when, placed at this menu's right
        // edge, it would run past the window's. Its width is the one it was
        // last drawn at, or this menu's own until it has been drawn.
        let submenu_width = self
            .submenu_index
            .and_then(|ix| match self.menu_items.get(ix) {
                Some(PopupMenuItem::Submenu { menu, .. }) => Some(menu.read(cx).bounds.size.width),
                _ => None,
            })
            .filter(|width| *width > Pixels::ZERO)
            .unwrap_or(bounds.size.width);
        let rem_size = window.rem_size();
        let (anchor, left) = if bounds.right() + submenu_width > window.viewport_size().width {
            (Anchor::TopRight, -rems(1.).to_pixels(rem_size))
        } else {
            (
                Anchor::TopLeft,
                bounds.size.width - rems(0.5).to_pixels(rem_size),
            )
        };

        let is_bottom_pos = bounds.origin.y + bounds.size.height > window.bounds().size.height;
        self.submenu_anchor = if is_bottom_pos {
            (anchor.other_side_along(gpui::Axis::Vertical), left)
        } else {
            (anchor, left)
        };
    }

    fn render_item(
        &self,
        ix: usize,
        item: &PopupMenuItem,
        options: RenderOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> MenuItemElement {
        let has_left_icon = options.has_left_icon;
        let is_left_check = options.check_side.is_left() && item.is_checked();
        let right_check_icon = if options.check_side.is_right() && item.is_checked() {
            Some(Icon::new(IconName::Check).xsmall())
        } else {
            None
        };

        let selected = self.selected_index == Some(ix);
        let submenu_open = self.submenu_index == Some(ix);
        const EDGE_PADDING: Pixels = px(4.);
        const INNER_PADDING: Rems = rems(0.5);

        let item_bounds = self.item_bounds.clone();
        let bounds_recorder = canvas(
            move |bounds, _, _| {
                if let Some(slot) = item_bounds.borrow_mut().get_mut(ix) {
                    *slot = bounds;
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();

        let (item_height, radius) = match self.size {
            Size::Small => (rems(1.25), options.radius.half()),
            _ => (rems(1.625), options.radius),
        };

        let this = MenuItemElement::new(ix)
            .relative()
            .text_sm()
            .py_0()
            .px(INNER_PADDING)
            .rounded(radius)
            .items_center()
            .selected(selected)
            .on_hover(cx.listener(move |this, hovered, window, cx| {
                this.on_item_hover(ix, *hovered, window, cx)
            }))
            .when_some(item.a11y_label(), |this, label| this.aria_label(label))
            .checked((item.is_checked() || self.checkable.contains(&ix)).then(|| item.is_checked()))
            .child(bounds_recorder);

        match item {
            PopupMenuItem::Separator => this
                .h_auto()
                .p_0()
                .my_0p5()
                .mx_neg_1()
                .border_b(px(2.))
                .border_color(cx.theme().border)
                .disabled(true),
            PopupMenuItem::Label(label) => this.disabled(true).cursor_default().child(
                h_flex()
                    .cursor_default()
                    .items_center()
                    .gap_x_1()
                    .children(Self::render_icon(has_left_icon, false, None, window, cx))
                    .child(div().flex_1().child(label.clone())),
            ),
            PopupMenuItem::ElementItem {
                render,
                icon,
                disabled,
                ..
            } => this
                .when(!disabled, |this| {
                    this.on_click(cx.listener(move |this, event, window, cx| {
                        this.on_click(ix, event, window, cx)
                    }))
                })
                .disabled(*disabled)
                .child(
                    h_flex()
                        .flex_1()
                        .min_h(item_height)
                        .items_center()
                        .gap_x_1()
                        .children(Self::render_icon(
                            has_left_icon,
                            is_left_check,
                            icon.clone(),
                            window,
                            cx,
                        ))
                        .child((render)(window, cx))
                        .children(right_check_icon.map(|icon| icon.ml_3())),
                ),
            PopupMenuItem::Item {
                icon,
                label,
                action,
                disabled,
                is_link,
                ..
            } => {
                let show_link_icon = *is_link && self.external_link_icon;
                let action = action.as_ref().map(|action| action.boxed_clone());
                let key = self.render_key_binding(action, window, cx);

                this.when(!disabled, |this| {
                    this.on_click(cx.listener(move |this, event, window, cx| {
                        this.on_click(ix, event, window, cx)
                    }))
                })
                .disabled(*disabled)
                .h(item_height)
                .gap_x_1()
                .children(Self::render_icon(
                    has_left_icon,
                    is_left_check,
                    icon.clone(),
                    window,
                    cx,
                ))
                .child(
                    h_flex()
                        .w_full()
                        .gap_3()
                        .items_center()
                        .justify_between()
                        .when(!show_link_icon, |this| this.child(label.clone()))
                        .children(right_check_icon)
                        .when(show_link_icon, |this| {
                            this.child(
                                h_flex()
                                    .w_full()
                                    .justify_between()
                                    .gap_1p5()
                                    .child(label.clone())
                                    .child(
                                        Icon::new(IconName::ExternalLink)
                                            .xsmall()
                                            .text_color(cx.theme().muted_foreground),
                                    ),
                            )
                        })
                        .children(key),
                )
            }
            PopupMenuItem::Submenu {
                icon,
                label,
                menu,
                disabled,
            } => this
                .when(!disabled, |this| {
                    this.on_click(cx.listener(move |this, event, window, cx| {
                        this.on_click(ix, event, window, cx)
                    }))
                })
                .selected(selected || submenu_open)
                .disabled(*disabled)
                .items_start()
                .child(
                    h_flex()
                        .min_h(item_height)
                        .size_full()
                        .items_center()
                        .gap_x_1()
                        .children(Self::render_icon(
                            has_left_icon,
                            false,
                            icon.clone(),
                            window,
                            cx,
                        ))
                        .child(
                            h_flex()
                                .flex_1()
                                .gap_2()
                                .items_center()
                                .justify_between()
                                .child(label.clone())
                                .child(
                                    Icon::new(IconName::ChevronRight)
                                        .xsmall()
                                        .text_color(cx.theme().muted_foreground),
                                ),
                        ),
                )
                .when(submenu_open, |this| {
                    this.child({
                        let (anchor, left) = self.submenu_anchor;
                        let is_bottom_pos =
                            matches!(anchor, Anchor::BottomLeft | Anchor::BottomRight);
                        deferred(
                            anchored()
                                .anchor(anchor)
                                .child(
                                    div()
                                        .id("submenu")
                                        .test_support()
                                        .occlude()
                                        .when(is_bottom_pos, |this| this.bottom_0())
                                        .when(!is_bottom_pos, |this| this.top_neg_1())
                                        .left(left)
                                        .child(menu.clone()),
                                )
                                .snap_to_window_with_margin(Edges::all(EDGE_PADDING)),
                        )
                        .with_priority(self.priority + 1)
                    })
                }),
        }
    }
}

impl FluentBuilder for PopupMenu {}
impl EventEmitter<DismissEvent> for PopupMenu {}
impl Focusable for PopupMenu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[derive(Clone, Copy)]
struct RenderOptions {
    has_left_icon: bool,
    check_side: Side,
    radius: Pixels,
}

impl Render for PopupMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.update_submenu_menu_anchor(window, cx);

        // Submenus attached via the public `item()` + `PopupMenuItem::submenu()`
        // path (from contexts that only have the menu value, e.g. a table
        // delegate's `context_menu`) have no parent wired at construction time.
        // Wire them here so the dismiss chain, click-outside checks and keyboard
        // navigation treat them the same as `submenu()`-built children.
        let parent = cx.entity().downgrade();
        let parent_priority = self.priority;
        for item in &self.menu_items {
            if let PopupMenuItem::Submenu { menu, .. } = item {
                if menu.read(cx).parent_menu.is_none() {
                    menu.update(cx, |menu, _| {
                        menu.parent_menu = Some(parent.clone());
                        menu.priority = parent_priority + 1;
                    });
                }
            }
        }

        self.scroll_to_checked_item(cx);

        let view = cx.entity().clone();
        let items_count = self.menu_items.len();
        self.item_bounds
            .borrow_mut()
            .resize(items_count, Bounds::default());
        let watches_pointer = self.submenu_index.is_some() || self.release_button.is_some();

        let max_height = self.max_height.unwrap_or_else(|| {
            let window_half_height = window.window_bounds().get_bounds().size.height * 0.5;
            window_half_height.min(rems(28.125).to_pixels(window.rem_size()))
        });

        let has_left_icon = self
            .menu_items
            .iter()
            .any(|item| item.has_left_icon(self.check_side));

        let options = RenderOptions {
            has_left_icon,
            check_side: self.check_side,
            radius: cx.theme().radius.min(px(8.)),
        };

        v_flex()
            .id("popup-menu")
            .test_support()
            .role(Role::Menu)
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_first))
            .on_action(cx.listener(Self::select_last))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::dismiss))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down_out(cx.listener(Self::on_mouse_down_out))
            .popover_style(cx)
            .text_color(cx.theme().popover_foreground)
            .relative()
            .occlude()
            .child(
                v_flex()
                    .id("items")
                    .p_1()
                    .gap_y_0p5()
                    .min_w(rems(8.))
                    .when_some(self.min_width, |this, min_width| this.min_w(min_width))
                    .map(|this| match self.max_width {
                        Some(max_width) => this.max_w(max_width),
                        None => this.max_w(rems(31.25)),
                    })
                    .when(self.scrollable, |this| {
                        this.max_h(max_height)
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll_handle)
                    })
                    .children(
                        self.menu_items
                            .iter()
                            .enumerate()
                            // Ignore last separator
                            .filter(|(ix, item)| !(*ix + 1 == items_count && item.is_separator()))
                            .map(|(ix, item)| self.render_item(ix, item, options, window, cx)),
                    )
                    .on_prepaint(move |bounds, _, cx| {
                        view.update(cx, |menu, cx| {
                            let width_changed = menu.bounds.size.width != bounds.size.width;
                            menu.bounds = bounds;
                            if width_changed {
                                if let Some(parent) = menu
                                    .parent_menu
                                    .as_ref()
                                    .and_then(|parent| parent.upgrade())
                                {
                                    parent.update(cx, |_, cx| cx.notify());
                                }
                            }
                        });
                    }),
            )
            .when(self.scrollable, |this| {
                this.vertical_scrollbar(&self.scroll_handle)
            })
            .when(watches_pointer, |this| {
                this.child(self.render_pointer_watcher(cx))
            })
    }
}

/// The press that opened the menu was released: the item under the pointer,
/// in this menu or one of its open submenus, is activated. Each menu is
/// updated on its own, so a submenu dismissing the chain can update its
/// parents.
fn release_press(
    root: &Entity<PopupMenu>,
    event: &MouseUpEvent,
    window: &mut Window,
    cx: &mut App,
) {
    // The listener may outlive the press it was painted for, in a frame
    // replayed from the cache, so the menu says whether it is still armed.
    if root.read(cx).release_button != Some(event.button) {
        return;
    }
    root.update(cx, |menu, _| menu.release_button = None);
    let mut menu = Some(root.clone());
    while let Some(current) = menu {
        if current.read(cx).hovered_index.is_some() {
            current.update(cx, |menu, cx| menu.release_on_hovered(event, window, cx));
            return;
        }
        menu = current.read(cx).active_submenu();
    }
}

#[cfg(test)]
impl PopupMenu {
    /// The highlighted item, for tests outside this module.
    pub(crate) fn highlighted(&self) -> Option<usize> {
        self.selected_index
    }

    /// Whether item `ix` shows a check mark, for tests outside this module.
    pub(crate) fn is_item_checked(&self, ix: usize) -> bool {
        self.menu_items
            .get(ix)
            .is_some_and(PopupMenuItem::is_checked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext, VisualTestContext, actions, point};
    use std::cell::Cell;

    actions!(popup_menu_test, [Noop]);

    /// Lets `ms` of timers run, then draws.
    fn settle(cx: &mut VisualTestContext, ms: u64) {
        cx.executor().advance_clock(Duration::from_millis(ms));
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn item_bounds(
        menu: &Entity<PopupMenu>,
        ix: usize,
        cx: &mut VisualTestContext,
    ) -> Bounds<Pixels> {
        menu.read_with(cx, |menu, _| menu.item_bounds.borrow()[ix])
    }

    /// The highlighted item and the item whose submenu is open.
    fn highlight_and_submenu(
        menu: &Entity<PopupMenu>,
        cx: &mut VisualTestContext,
    ) -> (Option<usize>, Option<usize>) {
        menu.read_with(cx, |menu, _| (menu.selected_index, menu.submenu_index))
    }

    /// A menu whose second item, "More", opens a submenu taller than the
    /// items below it.
    fn submenu_harness(cx: &mut TestAppContext) -> (Entity<PopupMenu>, &mut VisualTestContext) {
        cx.update(crate::init);
        let (menu, cx) = cx.add_window_view(|window, cx| {
            PopupMenu::new(cx)
                .menu("Alpha", Box::new(Noop))
                .submenu("More", window, cx, |menu, _, _| {
                    menu.menu("One", Box::new(Noop))
                        .menu("Two", Box::new(Noop))
                        .menu("Three", Box::new(Noop))
                        .menu("Four", Box::new(Noop))
                        .menu("Five", Box::new(Noop))
                })
                .menu("Beta", Box::new(Noop))
                .menu("Gamma", Box::new(Noop))
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        (menu, cx)
    }

    /// Hovers "More" until its submenu opens; returns the submenu's bounds.
    fn open_more(menu: &Entity<PopupMenu>, cx: &mut VisualTestContext) -> Bounds<Pixels> {
        let more = item_bounds(menu, 1, cx);
        cx.simulate_mouse_move(more.center(), None, Modifiers::default());
        settle(cx, 100);
        assert_eq!(highlight_and_submenu(menu, cx), (Some(1), Some(1)));
        menu.read_with(cx, |menu, cx| {
            menu.active_submenu()
                .expect("the submenu is open")
                .read(cx)
                .bounds
        })
    }

    /// A menu built by `build`, focused and drawn.
    fn focused_menu(
        cx: &mut TestAppContext,
        build: impl FnOnce(PopupMenu) -> PopupMenu + 'static,
    ) -> (Entity<PopupMenu>, &mut VisualTestContext) {
        cx.update(crate::init);
        let (menu, cx) = cx.add_window_view(move |_, cx| build(PopupMenu::new(cx)));
        cx.update(|window, cx| {
            let focus = menu.read(cx).focus_handle.clone();
            focus.focus(window, cx);
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        (menu, cx)
    }

    fn highlighted(menu: &Entity<PopupMenu>, cx: &mut VisualTestContext) -> Option<usize> {
        menu.read_with(cx, |menu, _| menu.selected_index)
    }

    fn fruit(menu: PopupMenu) -> PopupMenu {
        menu.menu("Apple", Box::new(Noop))
            .menu("Banana", Box::new(Noop))
            .menu("Blueberry", Box::new(Noop))
            .menu("Cherry", Box::new(Noop))
    }

    #[gpui::test]
    fn custom_menu_item_preserves_mouse_event(cx: &mut gpui::TestAppContext) {
        cx.update(crate::init);
        let events = Rc::new(RefCell::new(Vec::new()));
        let (_, cx) = cx.add_window_view({
            let events = events.clone();
            move |_, cx| {
                PopupMenu::new(cx).item(
                    PopupMenuItem::element(|_, _| {
                        div()
                            .debug_selector(|| "custom-menu-content".into())
                            .w(px(100.))
                            .h(px(20.))
                            .child("Custom")
                    })
                    .on_click(move |event, _, _| events.borrow_mut().push(event.clone())),
                )
            }
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let position = cx.debug_bounds("custom-menu-content").unwrap().center();
        let modifiers = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        cx.simulate_mouse_down(position, MouseButton::Left, modifiers);
        cx.simulate_event(MouseUpEvent {
            position,
            button: MouseButton::Left,
            modifiers,
            click_count: 2,
        });
        cx.run_until_parked();
        let events = events.borrow();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].mouse_position(), Some(position));
        assert_eq!(events[0].modifiers(), modifiers);
        let ClickEvent::Mouse(event) = &events[0] else {
            panic!("expected the original mouse click");
        };
        assert_eq!(event.up.click_count, 2);
    }

    #[gpui::test]
    fn popup_menu_item_a11y_label_uses_visible_label(cx: &mut gpui::TestAppContext) {
        let submenu = cx.update(|cx| cx.new(|cx| PopupMenu::new(cx)));

        assert_eq!(PopupMenuItem::new("Open").a11y_label(), Some("Open".into()));
        assert_eq!(
            PopupMenuItem::link("Docs", "https://example.com").a11y_label(),
            Some("Docs".into())
        );
        assert_eq!(
            PopupMenuItem::label("Recent files").a11y_label(),
            Some("Recent files".into())
        );
        assert_eq!(
            PopupMenuItem::submenu("More", submenu).a11y_label(),
            Some("More".into())
        );
        assert_eq!(PopupMenuItem::separator().a11y_label(), None);
        assert_eq!(PopupMenuItem::element(|_, _| div()).a11y_label(), None);
    }

    #[gpui::test]
    fn a_submenu_opens_after_its_item_is_hovered_for_100ms(cx: &mut TestAppContext) {
        let (menu, cx) = submenu_harness(cx);
        let more = item_bounds(&menu, 1, cx);

        cx.simulate_mouse_move(more.center(), None, Modifiers::default());
        settle(cx, 60);
        assert_eq!(
            highlight_and_submenu(&menu, cx),
            (Some(1), None),
            "the item is highlighted at once, its submenu not yet open"
        );

        settle(cx, 40);
        assert_eq!(highlight_and_submenu(&menu, cx), (Some(1), Some(1)));
    }

    #[gpui::test]
    fn moving_diagonally_across_a_sibling_toward_the_submenu_keeps_it_open(
        cx: &mut TestAppContext,
    ) {
        let (menu, cx) = submenu_harness(cx);
        let submenu = open_more(&menu, cx);
        let more = item_bounds(&menu, 1, cx);
        let beta = item_bounds(&menu, 2, cx);
        assert!(
            submenu.bottom() > beta.bottom(),
            "the submenu reaches below the sibling"
        );

        let start = more.center();
        let end = point(submenu.left(), submenu.bottom() - px(4.));
        let mut crossed_sibling = false;
        for step in 1..=6 {
            let t = step as f32 / 7.;
            let at = point(
                start.x + (end.x - start.x) * t,
                start.y + (end.y - start.y) * t,
            );
            crossed_sibling |= beta.contains(&at);
            cx.simulate_mouse_move(at, None, Modifiers::default());
            settle(cx, 30);
            assert_eq!(
                highlight_and_submenu(&menu, cx),
                (Some(1), Some(1)),
                "the sibling took over at step {step}"
            );
        }
        assert!(
            crossed_sibling,
            "the way to the submenu crosses the sibling"
        );

        cx.simulate_mouse_move(
            point(submenu.left() + px(10.), submenu.bottom() - px(10.)),
            None,
            Modifiers::default(),
        );
        settle(cx, 300);
        assert_eq!(highlight_and_submenu(&menu, cx), (Some(1), Some(1)));
    }

    #[gpui::test]
    fn resting_on_a_sibling_on_the_way_to_a_submenu_hands_it_the_highlight_after_100ms(
        cx: &mut TestAppContext,
    ) {
        let (menu, cx) = submenu_harness(cx);
        open_more(&menu, cx);
        let beta = item_bounds(&menu, 2, cx);

        cx.simulate_mouse_move(beta.center(), None, Modifiers::default());
        settle(cx, 60);
        assert_eq!(
            highlight_and_submenu(&menu, cx),
            (Some(1), Some(1)),
            "still on its way"
        );

        settle(cx, 40);
        assert_eq!(highlight_and_submenu(&menu, cx), (Some(2), None));
    }

    #[gpui::test]
    fn turning_away_from_a_submenu_onto_a_sibling_closes_it_at_once(cx: &mut TestAppContext) {
        let (menu, cx) = submenu_harness(cx);
        open_more(&menu, cx);
        let beta = item_bounds(&menu, 2, cx);
        let gamma = item_bounds(&menu, 3, cx);

        cx.simulate_mouse_move(
            point(beta.left() + px(40.), beta.center().y),
            None,
            Modifiers::default(),
        );
        settle(cx, 0);
        assert_eq!(highlight_and_submenu(&menu, cx), (Some(1), Some(1)));

        // Down and back, away from the submenu on the right.
        cx.simulate_mouse_move(
            point(gamma.left() + px(10.), gamma.center().y),
            None,
            Modifiers::default(),
        );
        settle(cx, 0);
        assert_eq!(highlight_and_submenu(&menu, cx), (Some(3), None));
    }

    #[gpui::test]
    fn typing_highlights_the_first_item_starting_with_the_typed_text(cx: &mut TestAppContext) {
        let (menu, cx) = focused_menu(cx, fruit);

        cx.simulate_keystrokes("b l");

        assert_eq!(highlighted(&menu, cx), Some(2), "Blueberry");
    }

    #[gpui::test]
    fn typing_one_letter_again_and_again_cycles_through_the_items_starting_with_it(
        cx: &mut TestAppContext,
    ) {
        let (menu, cx) = focused_menu(cx, fruit);

        cx.simulate_keystrokes("b");
        assert_eq!(highlighted(&menu, cx), Some(1), "Banana");
        cx.simulate_keystrokes("b");
        assert_eq!(highlighted(&menu, cx), Some(2), "Blueberry");
        cx.simulate_keystrokes("b");
        assert_eq!(highlighted(&menu, cx), Some(1), "back to Banana");
    }

    #[gpui::test]
    fn typing_skips_disabled_items_and_separators(cx: &mut TestAppContext) {
        let (menu, cx) = focused_menu(cx, |menu| {
            menu.menu("Apple", Box::new(Noop))
                .menu_with_disabled("Banana", Box::new(Noop), true)
                .separator()
                .menu("Blueberry", Box::new(Noop))
        });

        cx.simulate_keystrokes("b");

        assert_eq!(highlighted(&menu, cx), Some(3), "Blueberry");
    }

    #[gpui::test]
    fn typed_text_is_forgotten_after_half_a_second_without_typing(cx: &mut TestAppContext) {
        let (menu, cx) = focused_menu(cx, fruit);

        cx.simulate_keystrokes("b");
        assert_eq!(highlighted(&menu, cx), Some(1), "Banana");
        cx.executor().advance_clock(TYPEAHEAD_RESET_DELAY);
        cx.run_until_parked();
        // Typed within the delay, "a" would extend the search and stay on
        // Banana; after it, a fresh "a" goes to Apple.
        cx.simulate_keystrokes("a");

        assert_eq!(highlighted(&menu, cx), Some(0), "Apple");
    }

    #[gpui::test]
    fn home_and_end_highlight_the_first_and_last_enabled_items(cx: &mut TestAppContext) {
        let (menu, cx) = focused_menu(cx, |menu| {
            menu.menu_with_disabled("Zero", Box::new(Noop), true)
                .menu("One", Box::new(Noop))
                .menu("Two", Box::new(Noop))
                .menu_with_disabled("Three", Box::new(Noop), true)
        });

        cx.simulate_keystrokes("end");
        assert_eq!(highlighted(&menu, cx), Some(2));
        cx.simulate_keystrokes("home");
        assert_eq!(highlighted(&menu, cx), Some(1));
    }

    /// Counts the menu's dismissals.
    fn count_dismissals(
        menu: &Entity<PopupMenu>,
        cx: &mut VisualTestContext,
    ) -> (Rc<Cell<usize>>, Subscription) {
        let dismissed = Rc::new(Cell::new(0));
        let subscription = cx.update(|_, cx| {
            let dismissed = dismissed.clone();
            cx.subscribe(menu, move |_, _: &DismissEvent, _| {
                dismissed.set(dismissed.get() + 1)
            })
        });
        (dismissed, subscription)
    }

    #[gpui::test]
    fn choosing_an_option_leaves_the_menu_open(cx: &mut TestAppContext) {
        let (menu, cx) = focused_menu(cx, |menu| {
            menu.menu_with_check("Wrap lines", false, Box::new(Noop))
                .menu("Close", Box::new(Noop))
        });
        let (dismissed, _subscription) = count_dismissals(&menu, cx);

        cx.simulate_keystrokes("down enter");

        assert_eq!(dismissed.get(), 0);
        assert!(
            menu.read_with(cx, |menu, _| menu.is_stale()),
            "its owner is asked to show the new state"
        );
        cx.update(|window, cx| {
            assert!(
                menu.read(cx).focus_handle.is_focused(window),
                "it keeps the focus"
            );
        });
    }

    #[gpui::test]
    fn choosing_a_plain_item_closes_the_menu(cx: &mut TestAppContext) {
        let (menu, cx) = focused_menu(cx, |menu| {
            menu.menu_with_check("Wrap lines", false, Box::new(Noop))
                .menu("Close", Box::new(Noop))
        });
        let (dismissed, _subscription) = count_dismissals(&menu, cx);

        cx.simulate_keystrokes("down down enter");

        assert_eq!(dismissed.get(), 1);
    }
}
