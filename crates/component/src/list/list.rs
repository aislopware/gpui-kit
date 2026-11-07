use std::ops::Range;
use std::time::{Duration, Instant};

use crate::actions::{Cancel, Confirm, SelectDown, SelectFirst, SelectLast, SelectUp};
use crate::input::InputState;
use crate::list::cache::{MeasuredEntrySize, RowEntry, RowsCache};
use crate::{
    ActiveTheme, IconName, Size,
    input::{Input, InputEvent},
    scroll::Scrollbar,
    v_flex,
};
use crate::{Icon, IndexPath, Selectable, Sizable, StyledExt};
use crate::{VirtualListScrollHandle, list::ListDelegate, v_virtual_list};
use gpui::{
    App, AvailableSpace, ClickEvent, Context, DefiniteLength, EdgesRefinement, EventEmitter,
    KeyDownEvent, ListSizingBehavior, MouseMoveEvent, Pixels, Point, RenderOnce, Role,
    ScrollStrategy, SharedString, StatefulInteractiveElement, StyleRefinement, Subscription, px,
    size,
};
use gpui::{
    AppContext, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, KeyBinding,
    Length, MouseButton, ParentElement, Render, Styled, Task, Window, div, prelude::FluentBuilder,
};

pub(crate) fn init(cx: &mut App) {
    let context: Option<&str> = Some("List");
    cx.bind_keys([
        KeyBinding::new("escape", Cancel, context),
        KeyBinding::new("enter", Confirm { secondary: false }, context),
        KeyBinding::new(
            if cx.platform_info().is_mac() {
                "cmd-enter"
            } else {
                "ctrl-enter"
            },
            Confirm { secondary: true },
            context,
        ),
        KeyBinding::new("up", SelectUp, context),
        KeyBinding::new("down", SelectDown, context),
        KeyBinding::new("home", SelectFirst, context),
        KeyBinding::new("end", SelectLast, context),
    ]);
}

/// How long typeahead waits for the next key before a new query starts.
const TYPEAHEAD_TIMEOUT: Duration = Duration::from_millis(500);

/// The text typed so far to move the highlight to the item it begins.
#[derive(Default)]
struct Typeahead {
    /// The typed text, lowercased.
    query: String,
    /// The highlight when the query started; matches are searched after it.
    anchor: Option<IndexPath>,
    last_key_at: Option<Instant>,
}

type ItemLabel<D> = Box<dyn Fn(&D, IndexPath, &App) -> Option<SharedString> + 'static>;
type ItemEnabled<D> = Box<dyn Fn(&D, IndexPath, &App) -> bool + 'static>;

#[derive(Clone)]
pub enum ListEvent {
    /// Move to select item.
    Select(IndexPath),
    /// Click on item or pressed Enter.
    Confirm(IndexPath),
    /// Pressed ESC to deselect the item.
    Cancel,
}

struct ListOptions {
    scroll_bounce: Option<bool>,
    size: Size,
    scrollbar_visible: bool,
    search_placeholder: Option<SharedString>,
    max_height: Option<Length>,
    paddings: EdgesRefinement<DefiniteLength>,
}

impl Default for ListOptions {
    fn default() -> Self {
        Self {
            scroll_bounce: None,
            size: Size::default(),
            scrollbar_visible: true,
            max_height: None,
            search_placeholder: None,
            paddings: EdgesRefinement::default(),
        }
    }
}

/// The state for List.
///
/// List required all items has the same height.
pub struct ListState<D: ListDelegate> {
    pub(crate) focus_handle: FocusHandle,
    pub(crate) query_input: Entity<InputState>,
    options: ListOptions,
    delegate: D,
    /// The query the delegate was last asked to filter by, set as the search
    /// starts rather than when it lands, so a list that was never searched
    /// is known to be unfiltered.
    filtered_by: String,
    scroll_handle: VirtualListScrollHandle,
    rows_cache: RowsCache,
    selected_index: Option<IndexPath>,
    item_to_measure_index: IndexPath,
    deferred_scroll_to_index: Option<(IndexPath, ScrollStrategy)>,
    mouse_right_clicked_index: Option<IndexPath>,
    reset_on_cancel: bool,
    searchable: bool,
    selectable: bool,
    highlight_on_hover: bool,
    /// Where the pointer last was, so that only a pointer that really moved
    /// takes the highlight, never one the items scrolled or opened under.
    pointer_position: Option<Point<Pixels>>,
    typeahead: Typeahead,
    item_label: Option<ItemLabel<D>>,
    item_enabled: Option<ItemEnabled<D>>,
    _search_task: Task<()>,
    _load_more_task: Task<()>,
    _query_input_subscription: Subscription,
}

impl<D> ListState<D>
where
    D: ListDelegate,
{
    pub fn new(delegate: D, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search..."));

        let _query_input_subscription =
            cx.subscribe_in(&query_input, window, Self::on_query_input_event);

        Self {
            focus_handle: cx.focus_handle(),
            options: ListOptions::default(),
            delegate,
            rows_cache: RowsCache::default(),
            query_input,
            filtered_by: String::new(),
            selected_index: None,
            selectable: true,
            searchable: false,
            item_to_measure_index: IndexPath::default(),
            deferred_scroll_to_index: None,
            mouse_right_clicked_index: None,
            scroll_handle: VirtualListScrollHandle::new(),
            reset_on_cancel: true,
            highlight_on_hover: false,
            pointer_position: None,
            typeahead: Typeahead::default(),
            item_label: None,
            item_enabled: None,
            _search_task: Task::ready(()),
            _load_more_task: Task::ready(()),
            _query_input_subscription,
        }
    }

    /// Sets whether the list is searchable, default is `false`.
    ///
    /// When `true`, there will be a search input at the top of the list.
    pub fn searchable(mut self, searchable: bool) -> Self {
        self.searchable = searchable;
        self
    }

    pub fn set_searchable(&mut self, searchable: bool, cx: &mut Context<Self>) {
        if self.searchable != searchable {
            self.searchable = searchable;
            cx.notify();
        }
    }

    /// Sets whether the list is selectable, default is true.
    pub fn selectable(mut self, selectable: bool) -> Self {
        self.selectable = selectable;
        self
    }

    /// Sets whether the list is selectable, default is true.
    pub fn set_selectable(&mut self, selectable: bool, cx: &mut Context<Self>) {
        self.selectable = selectable;
        cx.notify();
    }

    /// Sets whether moving the pointer over an enabled item moves the
    /// highlight (the selected index) to it, default is `false`.
    ///
    /// Turn it on for a popup list, such as a select's or a completion menu's,
    /// where the highlight is a cursor that follows the pointer and arrow keys
    /// continue from wherever it is. Leave it off for a list whose selected
    /// row is a selection. Either way a list item paints no hover of its own,
    /// so there is only ever one highlighted row.
    pub fn highlight_on_hover(mut self, highlight_on_hover: bool) -> Self {
        self.highlight_on_hover = highlight_on_hover;
        self
    }

    /// Sets the text typeahead matches for each item.
    ///
    /// When the list is not searchable and has focus, typing moves the
    /// highlight to the next item whose label begins with the typed text, as
    /// a select does. Items without a label are skipped. Without this the list
    /// has no typeahead.
    pub fn item_label(
        mut self,
        label: impl Fn(&D, IndexPath, &App) -> Option<SharedString> + 'static,
    ) -> Self {
        self.item_label = Some(Box::new(label));
        self
    }

    /// Sets which items can be highlighted; every item can by default.
    ///
    /// Arrow keys, Home, End, typeahead and the pointer pass over an item
    /// this returns `false` for.
    pub fn item_enabled(mut self, enabled: impl Fn(&D, IndexPath, &App) -> bool + 'static) -> Self {
        self.item_enabled = Some(Box::new(enabled));
        self
    }

    pub fn delegate(&self) -> &D {
        &self.delegate
    }

    pub fn delegate_mut(&mut self) -> &mut D {
        &mut self.delegate
    }

    /// Focus the list, if the list is searchable, focus the search input.
    pub fn focus(&mut self, window: &mut Window, cx: &mut App) {
        self.focus_handle(cx).focus(window, cx);
    }

    /// Return true if either the list or the search input is focused.
    pub(crate) fn is_focused(&self, window: &Window, cx: &App) -> bool {
        self.focus_handle.is_focused(window) || self.query_input.focus_handle(cx).is_focused(window)
    }

    /// Set the selected index of the list,
    /// this will also scroll to the selected item.
    pub(crate) fn _set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selectable {
            return;
        }

        self.selected_index = ix;
        self.delegate.set_selected_index(ix, window, cx);
        self.scroll_to_selected_item(window, cx);
    }

    /// Set the selected index of the list,
    /// this method will not scroll to the selected item.
    pub fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected_index = ix;
        self.delegate.set_selected_index(ix, window, cx);
    }

    pub fn selected_index(&self) -> Option<IndexPath> {
        self.selected_index
    }

    /// Set the index of the item that has been right clicked.
    pub fn set_right_clicked_index(
        &mut self,
        ix: Option<IndexPath>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mouse_right_clicked_index = ix;
        self.delegate.set_right_clicked_index(ix, window, cx);
    }

    /// Returns the index of the item that has been right clicked.
    pub fn right_clicked_index(&self) -> Option<IndexPath> {
        self.mouse_right_clicked_index
    }

    /// Set the query text of the search input, this will trigger a search.
    pub fn set_query(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        let query = query.to_string();
        self.query_input.update(cx, |input, cx| {
            input.set_value(query.clone(), window, cx);
        });

        // `set_value` does not emit `InputEvent::Change`, so start the search here.
        self.start_search(query.trim().to_string(), window, cx);
    }

    /// Set a specific list item for measurement.
    ///
    /// If the item is absent, measure the first item in the first non-empty section.
    pub fn set_item_to_measure_index(
        &mut self,
        ix: IndexPath,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.item_to_measure_index = ix;
        cx.notify();
    }

    /// Scroll to the item at the given index.
    pub fn scroll_to_item(
        &mut self,
        ix: IndexPath,
        strategy: ScrollStrategy,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if ix.section == 0 && ix.row == 0 {
            // If the item is the first item, scroll to the top.
            let mut offset = self.scroll_handle.base_handle().offset();
            offset.y = px(0.);
            self.scroll_handle.base_handle().set_offset(offset);
            cx.notify();
            return;
        }
        self.deferred_scroll_to_index = Some((ix, strategy));
        cx.notify();
    }

    /// Get scroll handle
    pub fn scroll_handle(&self) -> &VirtualListScrollHandle {
        &self.scroll_handle
    }

    /// Scroll only as far as it takes to reveal the selected item.
    pub fn scroll_to_selected_item(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.selected_index {
            self.scroll_to_item(ix, ScrollStrategy::Nearest, window, cx);
        }
    }

    fn on_query_input_event(
        &mut self,
        state: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let text = state.read(cx).value();
                let text = text.trim().to_string();
                if text == self.filtered_by {
                    return;
                }

                self.start_search(text, window, cx);
            }
            _ => {}
        }
    }

    /// Filters the list by `query` without touching the search input, so a
    /// field can show one text while the list is filtered by another, as a
    /// combobox that shows its committed label shows every item.
    pub(crate) fn search(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        let query = query.trim().to_string();
        if self.filtered_by == query {
            return;
        }
        self.start_search(query, window, cx);
    }

    /// Where the item at `ix` sits among the rows as the list last laid them
    /// out: its top edge below the first row's top edge, its height, and the
    /// height of all the rows together. `None` for an item it does not show.
    pub(crate) fn item_extent(&self, ix: IndexPath) -> Option<(Pixels, Pixels, Pixels)> {
        let position = self.rows_cache.position_of(&ix)?;
        let sizes = &self.rows_cache.entries_sizes;
        let height = sizes.get(position)?.height;
        let top = sizes[..position].iter().map(|size| size.height).sum();
        let total = sizes.iter().map(|size| size.height).sum();
        Some((top, height, total))
    }

    /// Confirms the highlighted item, as Enter does in the list, for a field
    /// that keeps the keyboard focus while the list is open.
    pub(crate) fn confirm_highlighted(
        &mut self,
        secondary: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_action_confirm(&Confirm { secondary }, window, cx);
    }

    fn start_search(&mut self, query: String, window: &mut Window, cx: &mut Context<Self>) {
        self.set_searching(true, window, cx);
        let search = self.delegate.perform_search(&query, window, cx);
        self.filtered_by = query;

        if self.rows_cache.len() > 0 {
            self._set_selected_index(Some(IndexPath::default()), window, cx);
        } else {
            self._set_selected_index(None, window, cx);
        }

        self._search_task = cx.spawn_in(window, async move |this, window| {
            search.await;

            _ = this.update_in(window, |this, _, _| {
                this.scroll_handle.scroll_to_item(0, ScrollStrategy::Top);
            });

            // Always wait 100ms to avoid flicker
            window
                .background_executor()
                .timer(Duration::from_millis(100))
                .await;
            _ = this.update_in(window, |this, window, cx| {
                this.set_searching(false, window, cx);
            });
        });
    }

    fn set_searching(&mut self, searching: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.query_input
            .update(cx, |input, cx| input.set_loading(searching, window, cx));
    }

    /// Dispatch delegate's `load_more` method when the
    /// visible range is near the end.
    fn load_more_if_need(
        &mut self,
        entities_count: usize,
        visible_end: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // FIXME: Here need void sections items count.

        let threshold = self.delegate.load_more_threshold();
        // Securely handle subtract logic to prevent attempt
        // to subtract with overflow
        if visible_end >= entities_count.saturating_sub(threshold) {
            if !self.delegate.has_more(cx) {
                return;
            }

            self._load_more_task = cx.spawn_in(window, async move |view, cx| {
                _ = view.update_in(cx, |view, window, cx| {
                    view.delegate.load_more(window, cx);
                });
            });
        }
    }

    pub(crate) fn reset_on_cancel(mut self, reset: bool) -> Self {
        self.reset_on_cancel = reset;
        self
    }

    fn on_action_cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        cx.propagate();
        if self.reset_on_cancel {
            self._set_selected_index(None, window, cx);
        }

        self.delegate.cancel(window, cx);
        cx.emit(ListEvent::Cancel);
        cx.notify();
    }

    fn on_action_confirm(
        &mut self,
        confirm: &Confirm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.rows_cache.len() == 0 {
            return;
        }

        let Some(ix) = self.selected_index else {
            return;
        };

        self.delegate
            .set_selected_index(self.selected_index, window, cx);
        self.delegate.confirm(confirm.secondary, window, cx);
        cx.emit(ListEvent::Confirm(ix));
        cx.notify();
    }

    fn select_item(&mut self, ix: IndexPath, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selectable {
            return;
        }

        self.selected_index = Some(ix);
        self.delegate.set_selected_index(Some(ix), window, cx);
        self.scroll_to_selected_item(window, cx);
        cx.emit(ListEvent::Select(ix));
        cx.notify();
    }

    pub(crate) fn on_action_select_prev(
        &mut self,
        _: &SelectUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_highlight(false, window, cx);
    }

    pub(crate) fn on_action_select_next(
        &mut self,
        _: &SelectDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_highlight(true, window, cx);
    }

    /// Move the highlight to the next enabled item, wrapping around, from
    /// wherever the highlight is.
    pub(crate) fn step_highlight(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut ix = self.selected_index;
        for _ in 0..self.rows_cache.items_count() {
            let next = if forward {
                self.rows_cache.next(ix)
            } else {
                self.rows_cache.prev(ix)
            };
            if self.is_item_enabled(next, cx) {
                self.select_item(next, window, cx);
                return;
            }
            ix = Some(next);
        }
    }

    pub(crate) fn on_action_select_first(
        &mut self,
        _: &SelectFirst,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ix) = self
            .item_paths(cx)
            .into_iter()
            .find(|ix| self.is_item_enabled(*ix, cx))
        {
            self.select_item(ix, window, cx);
        }
    }

    pub(crate) fn on_action_select_last(
        &mut self,
        _: &SelectLast,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ix) = self
            .item_paths(cx)
            .into_iter()
            .rfind(|ix| self.is_item_enabled(*ix, cx))
        {
            self.select_item(ix, window, cx);
        }
    }

    /// Every item, in display order, as the delegate has them now; unlike the
    /// rows cache this does not wait for the list to be drawn.
    fn item_paths(&self, cx: &App) -> Vec<IndexPath> {
        (0..self.delegate.sections_count(cx).max(1))
            .flat_map(|section| {
                (0..self.delegate.items_count(section, cx))
                    .map(move |row| IndexPath::new(row).section(section))
            })
            .collect()
    }

    fn is_item_enabled(&self, ix: IndexPath, cx: &App) -> bool {
        self.item_enabled
            .as_ref()
            .is_none_or(|enabled| enabled(&self.delegate, ix, cx))
    }

    /// Add `character`, typed at `now`, to the typeahead query and return the
    /// enabled item the query now begins, if any. The highlight is left alone.
    ///
    /// A query continues while keys come within [`TYPEAHEAD_TIMEOUT`] of each
    /// other and is matched from the item after the highlight it started at,
    /// so typing the start of the highlighted item's label moves on to the
    /// next item that begins with it. Repeating one character cycles through
    /// the items that begin with it.
    pub(crate) fn typeahead(
        &mut self,
        character: char,
        now: Instant,
        cx: &App,
    ) -> Option<IndexPath> {
        if self.item_label.is_none() {
            return None;
        }
        let expired = self
            .typeahead
            .last_key_at
            .is_none_or(|at| now.saturating_duration_since(at) > TYPEAHEAD_TIMEOUT);
        if expired {
            self.typeahead.query.clear();
        }
        if self.typeahead.query.is_empty() && character.is_whitespace() {
            return None;
        }
        self.typeahead.last_key_at = Some(now);

        let typed: String = character.to_lowercase().collect();
        let repeats = !self.typeahead.query.is_empty()
            && self
                .typeahead
                .query
                .trim_start_matches(typed.as_str())
                .is_empty();
        if self.typeahead.query.is_empty() || repeats {
            self.typeahead.query.clear();
            self.typeahead.anchor = self.selected_index;
        }
        self.typeahead.query.push_str(&typed);

        let items = self.item_paths(cx);
        let start = self
            .typeahead
            .anchor
            .and_then(|anchor| items.iter().position(|ix| ix.eq_row(anchor)))
            .map_or(0, |position| position + 1);
        let query = self.typeahead.query.as_str();
        let item_label = self.item_label.as_ref()?;
        (0..items.len())
            .map(|offset| items[(start + offset) % items.len()])
            .find(|ix| {
                self.is_item_enabled(*ix, cx)
                    && item_label(&self.delegate, *ix, cx)
                        .is_some_and(|label| label.to_lowercase().starts_with(query))
            })
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(character) = gpui_base::Select::typeahead_character(&event.keystroke) else {
            return;
        };
        cx.stop_propagation();
        let now = cx.background_executor().now();
        if let Some(ix) = self.typeahead(character, now, cx) {
            self.select_item(ix, window, cx);
        }
    }

    /// Move the highlight to the item under a pointer that really moved.
    fn highlight_on_pointer(
        &mut self,
        ix: IndexPath,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let moved = self.pointer_position.replace(event.position) != Some(event.position);
        if !moved
            || !self.highlight_on_hover
            || !self.selectable
            || self
                .selected_index
                .is_some_and(|selected| selected.eq_row(ix))
            || !self.is_item_enabled(ix, cx)
        {
            return;
        }

        self.selected_index = Some(ix);
        self.delegate.set_selected_index(Some(ix), window, cx);
        cx.emit(ListEvent::Select(ix));
        cx.notify();
    }

    fn prepare_items_if_needed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sections_count = self.delegate.sections_count(cx).max(1);
        let mut measured_size = MeasuredEntrySize::default();

        // Measure the item_height and section header/footer height.
        let available_space = size(AvailableSpace::MinContent, AvailableSpace::MinContent);
        // Use the fallback for this measurement without overwriting the caller's configured index.
        let requested = self.item_to_measure_index;
        let item_to_measure = if requested.section < sections_count
            && requested.row < self.delegate.items_count(requested.section, cx)
        {
            Some(requested)
        } else {
            (0..sections_count)
                .find(|section| self.delegate.items_count(*section, cx) > 0)
                .map(|section| IndexPath::default().section(section))
        };
        if let Some(index) = item_to_measure {
            measured_size.item_size = self
                .render_list_item(index, window, cx)
                .into_any_element()
                .layout_as_root(available_space, window, cx);
        }

        if let Some(mut el) = self
            .delegate
            .render_section_header(0, window, cx)
            .map(|r| r.into_any_element())
        {
            measured_size.section_header_size = el.layout_as_root(available_space, window, cx);
        }
        if let Some(mut el) = self
            .delegate
            .render_section_footer(0, window, cx)
            .map(|r| r.into_any_element())
        {
            measured_size.section_footer_size = el.layout_as_root(available_space, window, cx);
        }

        self.rows_cache
            .prepare_if_needed(sections_count, measured_size, cx, |section_ix, cx| {
                self.delegate.items_count(section_ix, cx)
            });
    }

    fn render_list_item(
        &mut self,
        ix: IndexPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selectable = self.selectable;
        let selected = self.selected_index.map(|s| s.eq_row(ix)).unwrap_or(false);
        let mouse_right_clicked = self
            .mouse_right_clicked_index
            .map(|s| s.eq_row(ix))
            .unwrap_or(false);
        let id = SharedString::from(format!("list-item-{}", ix));

        let total_items = self.rows_cache.items_count();

        div()
            .id(id)
            .debug_selector(|| format!("list-item-{}-{}", ix.section, ix.row))
            .role(Role::ListItem)
            .aria_position_in_set(ix.row + 1)
            .aria_size_of_set(total_items)
            .aria_selected(selected)
            .w_full()
            .relative()
            .overflow_hidden()
            .children(self.delegate.render_item(ix, window, cx).map(|item| {
                item.selected(selected)
                    .secondary_selected(mouse_right_clicked)
            }))
            .when(selectable, |this| {
                this.on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, window, cx| {
                    this.highlight_on_pointer(ix, e, window, cx);
                }))
                .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                    this.set_right_clicked_index(None, window, cx);
                    this.selected_index = Some(ix);
                    this.on_action_confirm(
                        &Confirm {
                            secondary: e.modifiers().secondary(cx),
                        },
                        window,
                        cx,
                    );
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, _, window, cx| {
                        this.set_right_clicked_index(Some(ix), window, cx);
                        cx.notify();
                    }),
                )
            })
    }

    fn render_items(
        &mut self,
        items_count: usize,
        entities_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let rows_cache = self.rows_cache.clone();
        let scrollbar_visible = self.options.scrollbar_visible;
        let scroll_handle = self.scroll_handle.clone();
        let item_to_measure_index = rows_cache
            .position_of(&self.item_to_measure_index)
            .or_else(|| rows_cache.first_entry_position())
            .unwrap_or(0);

        v_flex()
            .flex_grow_1()
            .relative()
            .size_full()
            .when_some(self.options.max_height, |this, h| this.max_h(h))
            .overflow_hidden()
            .when(items_count == 0, |this| {
                this.child(self.delegate.render_empty(window, cx))
            })
            .when(items_count > 0, {
                |this| {
                    this.child(
                        v_virtual_list(
                            cx.entity(),
                            "virtual-list",
                            rows_cache.entries_sizes.clone(),
                            move |list, visible_range: Range<usize>, window, cx| {
                                list.load_more_if_need(
                                    entities_count,
                                    visible_range.end,
                                    window,
                                    cx,
                                );

                                // NOTE: Here the v_virtual_list would not able to have gap_y,
                                // because the section header, footer is always have rendered as a empty child item,
                                // even the delegate give a None result.

                                visible_range
                                    .map(|ix| {
                                        let Some(entry) = rows_cache.get(ix) else {
                                            return div();
                                        };

                                        div().children(match entry {
                                            RowEntry::Entry(index) => Some(
                                                list.render_list_item(index, window, cx)
                                                    .into_any_element(),
                                            ),
                                            RowEntry::SectionHeader(section_ix) => list
                                                .delegate_mut()
                                                .render_section_header(section_ix, window, cx)
                                                .map(|r| r.into_any_element()),
                                            RowEntry::SectionFooter(section_ix) => list
                                                .delegate_mut()
                                                .render_section_footer(section_ix, window, cx)
                                                .map(|r| r.into_any_element()),
                                        })
                                    })
                                    .collect::<Vec<_>>()
                            },
                        )
                        .with_item_to_measure_index(item_to_measure_index)
                        .paddings(self.options.paddings.clone())
                        .when(self.options.max_height.is_some(), |this| {
                            this.with_sizing_behavior(ListSizingBehavior::Infer)
                        })
                        .track_scroll(&scroll_handle)
                        .map(|list| {
                            gpui_base::ScrollBounce::new("list-scroll-bounce", &scroll_handle, list)
                                .when_some(self.options.scroll_bounce, |bounce, on| {
                                    bounce.enabled(on)
                                })
                        })
                        .into_any_element(),
                    )
                }
            })
            .when(scrollbar_visible, |this| {
                this.child(Scrollbar::vertical(&scroll_handle).viewport_from_layout())
            })
    }
}

impl<D> Focusable for ListState<D>
where
    D: ListDelegate,
{
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.searchable {
            self.query_input.focus_handle(cx)
        } else {
            self.focus_handle.clone()
        }
    }
}
impl<D> EventEmitter<ListEvent> for ListState<D> where D: ListDelegate {}
impl<D> Render for ListState<D>
where
    D: ListDelegate,
{
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.prepare_items_if_needed(window, cx);
        // Items that open or scroll under a still pointer are not hovered by
        // a move; only a pointer leaving this position takes the highlight.
        self.pointer_position = Some(window.mouse_position());
        let typeahead = !self.searchable && self.item_label.is_some();

        // Scroll to the selected item if it is set.
        if let Some((ix, strategy)) = self.deferred_scroll_to_index.take() {
            if let Some(item_ix) = self.rows_cache.position_of(&ix) {
                self.scroll_handle.scroll_to_item(item_ix, strategy);
            }
        }

        let loading = self.delegate().loading(cx);
        let query_input = if self.searchable {
            // sync placeholder
            if let Some(placeholder) = &self.options.search_placeholder {
                self.query_input.update(cx, |input, cx| {
                    input.set_placeholder(placeholder.clone(), window, cx);
                });
            }
            Some(self.query_input.clone())
        } else {
            None
        };

        let loading_view = if loading {
            Some(self.delegate.render_loading(window, cx).into_any_element())
        } else {
            None
        };
        let initial_view = if let Some(input) = &query_input {
            if input.read(cx).value().is_empty() {
                self.delegate.render_initial(window, cx)
            } else {
                None
            }
        } else {
            None
        };
        let items_count = self.rows_cache.items_count();
        let entities_count = self.rows_cache.len();
        let mouse_right_clicked_index = self.mouse_right_clicked_index;

        v_flex()
            .key_context("List")
            .id("list-state")
            .track_focus(&self.focus_handle)
            .role(Role::List)
            .size_full()
            .relative()
            .overflow_hidden()
            .when_some(query_input, |this, input| {
                this.child(
                    div()
                        .map(|this| match self.options.size {
                            Size::Small => this.px_1p5(),
                            _ => this.px_2(),
                        })
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            Input::new(&input)
                                .with_size(self.options.size)
                                .prefix(
                                    Icon::new(IconName::Search)
                                        .text_color(cx.theme().muted_foreground),
                                )
                                .cleanable(true)
                                .p_0()
                                .appearance(false),
                        ),
                )
            })
            .when(!loading, |this| {
                this.on_action(cx.listener(Self::on_action_cancel))
                    .on_action(cx.listener(Self::on_action_confirm))
                    .on_action(cx.listener(Self::on_action_select_next))
                    .on_action(cx.listener(Self::on_action_select_prev))
                    .on_action(cx.listener(Self::on_action_select_first))
                    .on_action(cx.listener(Self::on_action_select_last))
                    .when(typeahead, |this| {
                        this.on_key_down(cx.listener(Self::on_key_down))
                    })
                    .map(|this| {
                        if let Some(view) = initial_view {
                            this.child(view)
                        } else {
                            this.child(self.render_items(items_count, entities_count, window, cx))
                        }
                    })
                    // Click out to cancel right clicked row
                    .when(mouse_right_clicked_index.is_some(), |this| {
                        this.on_mouse_down_out(cx.listener(|this, _, window, cx| {
                            this.set_right_clicked_index(None, window, cx);
                            cx.notify();
                        }))
                    })
            })
            .children(loading_view)
    }
}

/// The List element.
#[derive(IntoElement)]
pub struct List<D: ListDelegate + 'static> {
    state: Entity<ListState<D>>,
    style: StyleRefinement,
    options: ListOptions,
}

impl<D> List<D>
where
    D: ListDelegate + 'static,
{
    /// Create a new List element with the given ListState entity.
    pub fn new(state: &Entity<ListState<D>>) -> Self {
        Self {
            state: state.clone(),
            style: StyleRefinement::default(),
            options: ListOptions::default(),
        }
    }

    /// Set whether the scrollbar is visible, default is `true`.
    pub fn scrollbar_visible(mut self, visible: bool) -> Self {
        self.options.scrollbar_visible = visible;
        self
    }

    /// Bounce at the vertical boundaries. On by default; reduced motion
    /// disables displacement.
    pub fn scroll_bounce(mut self, enabled: bool) -> Self {
        self.options.scroll_bounce = Some(enabled);
        self
    }

    /// Sets the placeholder text for the search input.
    pub fn search_placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.options.search_placeholder = Some(placeholder.into());
        self
    }
}

impl<D> Styled for List<D>
where
    D: ListDelegate + 'static,
{
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl<D> Sizable for List<D>
where
    D: ListDelegate + 'static,
{
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.options.size = size.into();
        self
    }
}

impl<D> RenderOnce for List<D>
where
    D: ListDelegate + 'static,
{
    fn render(mut self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        // Take paddings, max_height to options, and clear them from style,
        // because they would be applied to the inner virtual list.
        self.options.paddings = self.style.padding.clone();
        self.options.max_height = self.style.max_size.height;
        self.style.padding = EdgesRefinement::default();
        self.style.max_size.height = None;

        self.state.update(cx, |state, _| {
            state.options = self.options;
        });

        div()
            .id("list")
            .size_full()
            .refine_style(&self.style)
            .child(self.state.clone())
    }
}

#[cfg(test)]
mod measurement_tests {
    use super::*;
    use crate::list::ListItem;
    use gpui::{Element, TestAppContext};

    struct Delegate {
        counts: Vec<usize>,
    }

    impl ListDelegate for Delegate {
        type Item = ListItem;
        fn sections_count(&self, _: &App) -> usize {
            self.counts.len()
        }
        fn items_count(&self, section: usize, _: &App) -> usize {
            self.counts[section]
        }
        fn set_selected_index(
            &mut self,
            _: Option<IndexPath>,
            _: &mut Window,
            _: &mut Context<ListState<Self>>,
        ) {
        }
        fn render_item(
            &mut self,
            index: IndexPath,
            _: &mut Window,
            _: &mut Context<ListState<Self>>,
        ) -> Option<ListItem> {
            (index.row < *self.counts.get(index.section)?)
                .then(|| ListItem::new(index.row).h(px(if index.row == 0 { 36. } else { 48. })))
        }
    }

    struct BounceList(Entity<ListState<Delegate>>);

    impl Render for BounceList {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(240.))
                .h(px(200.))
                .child(List::new(&self.0).scroll_bounce(true).size_full())
        }
    }

    #[gpui::test]
    fn scroll_bounce_list_accepts_a_reverse_wheel_during_return(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            BounceList(cx.new(|cx| ListState::new(Delegate { counts: vec![40] }, window, cx)))
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let state = view.read_with(cx, |view, _| view.0.clone());
        let wheel = |cx: &mut gpui::VisualTestContext, dy| {
            cx.simulate_event(gpui::ScrollEvent {
                position: gpui::point(px(100.), px(100.)),
                delta: gpui::ScrollDelta::Lines(gpui::point(0., dy)),
                ..Default::default()
            });
        };
        for (dy, phase) in [
            (-10000., gpui::TouchPhase::Started),
            (0., gpui::TouchPhase::Ended),
        ] {
            cx.simulate_event(gpui::ScrollEvent {
                position: gpui::point(px(100.), px(100.)),
                delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(dy))),
                touch_phase: phase,
                ..Default::default()
            });
        }
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let end = state.read_with(cx, |state, _| state.scroll_handle.offset().y);
        assert!(end < px(0.));
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(200));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        wheel(cx, 2.);
        assert!(state.read_with(cx, |state, _| state.scroll_handle.offset().y) > end);
    }

    #[gpui::test]
    fn measures_an_existing_row_when_the_requested_item_is_absent(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let window = cx.add_empty_window();
        window.draw(
            gpui::point(px(0.), px(0.)),
            size(px(300.), px(300.)),
            |window, cx| {
                let list = cx.new(|cx| ListState::new(Delegate { counts: vec![0, 2] }, window, cx));
                list.update(cx, |list, cx| {
                    for (requested, expected_height) in [
                        (IndexPath::default(), 36.),
                        (IndexPath::new(1).section(1), 48.),
                        (IndexPath::new(99).section(1), 36.),
                        (IndexPath::new(0).section(99), 36.),
                    ] {
                        list.set_item_to_measure_index(requested, window, cx);
                        list.prepare_items_if_needed(window, cx);
                        let position = list
                            .rows_cache
                            .position_of(&IndexPath::new(0).section(1))
                            .unwrap();
                        assert_eq!(
                            list.rows_cache.entries_sizes[position].height,
                            px(expected_height)
                        );
                        assert_eq!(list.item_to_measure_index, requested);
                    }
                    let requested = IndexPath::new(1).section(1);
                    list.set_item_to_measure_index(requested, window, cx);
                    // Filtering removes the requested row, then all rows, before restoring it.
                    for (counts, expected_height) in [
                        (vec![0, 2], Some(48.)),
                        (vec![0, 1], Some(36.)),
                        (vec![0, 0], None),
                        (vec![0, 2], Some(48.)),
                    ] {
                        list.delegate.counts = counts;
                        list.prepare_items_if_needed(window, cx);
                        if let Some(height) = expected_height {
                            let position = list
                                .rows_cache
                                .position_of(&IndexPath::new(0).section(1))
                                .unwrap();
                            assert_eq!(list.rows_cache.entries_sizes[position].height, px(height));
                        } else {
                            assert_eq!(list.rows_cache.items_count(), 0);
                            assert!(list.rows_cache.entries_sizes.is_empty());
                        }
                        assert_eq!(list.item_to_measure_index, requested);
                    }
                });
                div()
            },
        );
    }

    #[gpui::test]
    fn list_state_has_list_role(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let window = cx.add_empty_window();
        window.draw(
            gpui::point(px(0.), px(0.)),
            size(px(300.), px(300.)),
            |window, cx| {
                let list = cx.new(|cx| ListState::new(Delegate { counts: vec![2] }, window, cx));
                list.update(cx, |list, cx| {
                    let element = list.render(window, cx).into_element();
                    assert_eq!(element.a11y_role(), Some(Role::List));
                });
                div()
            },
        );
    }
}

#[cfg(test)]
mod highlight_tests {
    use super::*;
    use crate::list::ListItem;
    use gpui::{Modifiers, TestAppContext, VisualTestContext, point};

    const FRUITS: [&str; 6] = [
        "Apple",
        "Banana",
        "Blueberry",
        "Cherry",
        "Coconut",
        "Cranberry",
    ];
    const ROW: f32 = 30.;

    struct Fruits {
        disabled: Vec<usize>,
    }

    impl ListDelegate for Fruits {
        type Item = ListItem;
        fn items_count(&self, _: usize, _: &App) -> usize {
            FRUITS.len()
        }
        fn set_selected_index(
            &mut self,
            _: Option<IndexPath>,
            _: &mut Window,
            _: &mut Context<ListState<Self>>,
        ) {
        }
        fn render_item(
            &mut self,
            ix: IndexPath,
            _: &mut Window,
            _: &mut Context<ListState<Self>>,
        ) -> Option<ListItem> {
            Some(ListItem::new(ix.row).h(px(ROW)).child(FRUITS[ix.row]))
        }
    }

    /// A popup-style list, three and a bit rows tall, at the window's origin.
    struct Popup(Entity<ListState<Fruits>>);

    impl Render for Popup {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(200.))
                .h(px(100.))
                .child(List::new(&self.0).size_full())
        }
    }

    fn popup(
        cx: &mut TestAppContext,
        disabled: Vec<usize>,
    ) -> (Entity<ListState<Fruits>>, &mut VisualTestContext) {
        cx.update(crate::init);
        let (popup, cx) = cx.add_window_view(|window, cx| {
            Popup(cx.new(|cx| {
                ListState::new(Fruits { disabled }, window, cx)
                    .highlight_on_hover(true)
                    .item_label(|_, ix, _| FRUITS.get(ix.row).map(|&fruit| fruit.into()))
                    .item_enabled(|fruits, ix, _| !fruits.disabled.contains(&ix.row))
            }))
        });
        let list = popup.read_with(cx, |popup, _| popup.0.clone());
        cx.update(|window, cx| {
            list.read(cx).focus_handle(cx).focus(window, cx);
            window.draw(cx).clear(cx);
        });
        (list, cx)
    }

    fn draw(cx: &mut VisualTestContext) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn highlighted(list: &Entity<ListState<Fruits>>, cx: &mut VisualTestContext) -> Option<usize> {
        list.read_with(cx, |list, _| list.selected_index().map(|ix| ix.row))
    }

    /// The pointer at `x` across the row at `row`, with the list unscrolled.
    fn pointer_at(cx: &mut VisualTestContext, row: usize, x: f32) {
        let y = ROW * row as f32 + ROW / 2.;
        cx.simulate_mouse_move(point(px(x), px(y)), None, Modifiers::none());
    }

    #[gpui::test]
    fn the_pointer_moves_the_one_highlight_and_keys_continue_from_it(cx: &mut TestAppContext) {
        let (list, cx) = popup(cx, vec![]);
        cx.simulate_keystrokes("down");
        assert_eq!(highlighted(&list, cx), Some(0));
        draw(cx);

        pointer_at(cx, 2, 100.);
        assert_eq!(
            highlighted(&list, cx),
            Some(2),
            "the highlight leaves `Apple`"
        );
        draw(cx);
        cx.simulate_keystrokes("up");
        assert_eq!(
            highlighted(&list, cx),
            Some(1),
            "`up` goes on from `Blueberry`"
        );
    }

    #[gpui::test]
    fn a_pointer_that_has_not_moved_does_not_take_the_highlight(cx: &mut TestAppContext) {
        let (list, cx) = popup(cx, vec![]);
        pointer_at(cx, 1, 100.);
        assert_eq!(highlighted(&list, cx), Some(1));
        draw(cx);
        cx.simulate_keystrokes("down");
        draw(cx);

        // A move to where the pointer already is, as a platform reports when
        // the items scroll or open under a still pointer.
        pointer_at(cx, 1, 100.);
        assert_eq!(highlighted(&list, cx), Some(2));
        pointer_at(cx, 1, 101.);
        assert_eq!(highlighted(&list, cx), Some(1), "a real move takes it back");
    }

    #[gpui::test]
    fn the_pointer_passes_over_a_disabled_item(cx: &mut TestAppContext) {
        let (list, cx) = popup(cx, vec![1]);
        pointer_at(cx, 0, 100.);
        draw(cx);
        pointer_at(cx, 1, 100.);
        assert_eq!(highlighted(&list, cx), Some(0));
    }

    #[gpui::test]
    fn arrow_keys_skip_disabled_items(cx: &mut TestAppContext) {
        let (list, cx) = popup(cx, vec![1, 5]);
        cx.simulate_keystrokes("down down");
        assert_eq!(highlighted(&list, cx), Some(2));
        cx.simulate_keystrokes("up up");
        assert_eq!(
            highlighted(&list, cx),
            Some(4),
            "`up` wraps past `Cranberry`"
        );
    }

    #[gpui::test]
    fn home_and_end_highlight_the_first_and_last_enabled_items(cx: &mut TestAppContext) {
        let (list, cx) = popup(cx, vec![0, 5]);
        cx.simulate_keystrokes("end");
        assert_eq!(highlighted(&list, cx), Some(4));
        cx.simulate_keystrokes("home");
        assert_eq!(highlighted(&list, cx), Some(1));
    }

    #[gpui::test]
    fn arrow_keys_scroll_only_as_far_as_the_highlight_needs(cx: &mut TestAppContext) {
        let (list, cx) = popup(cx, vec![]);
        let offset = |cx: &mut VisualTestContext| {
            draw(cx);
            list.read_with(cx, |list, _| list.scroll_handle().offset().y)
        };
        cx.simulate_keystrokes("down down down");
        assert_eq!(offset(cx), px(0.), "`Blueberry` is already in view");
        cx.simulate_keystrokes("down");
        let scrolled = offset(cx);
        assert!(
            scrolled < px(0.) && scrolled > px(-ROW * 2.),
            "`Cherry` is revealed at the bottom, not jumped to the top: {scrolled:?}"
        );
    }

    #[gpui::test]
    fn typing_highlights_the_next_item_that_begins_with_the_typed_text(cx: &mut TestAppContext) {
        let (list, cx) = popup(cx, vec![]);
        cx.simulate_keystrokes("b->b");
        assert_eq!(highlighted(&list, cx), Some(1));
        cx.simulate_keystrokes("L->L");
        assert_eq!(highlighted(&list, cx), Some(2), "`bl` begins `Blueberry`");
        cx.simulate_keystrokes("x->x");
        assert_eq!(highlighted(&list, cx), Some(2), "`blx` begins nothing");
    }

    #[gpui::test]
    fn typeahead_starts_over_half_a_second_after_the_last_key(cx: &mut TestAppContext) {
        let (list, cx) = popup(cx, vec![]);
        cx.simulate_keystrokes("c->c");
        cx.executor().advance_clock(Duration::from_millis(400));
        cx.simulate_keystrokes("o->o");
        assert_eq!(highlighted(&list, cx), Some(4), "`co` begins `Coconut`");

        cx.executor().advance_clock(Duration::from_millis(600));
        cx.simulate_keystrokes("a->a");
        assert_eq!(
            highlighted(&list, cx),
            Some(0),
            "a fresh `a` begins `Apple`"
        );
    }

    #[gpui::test]
    fn repeating_a_character_cycles_through_the_items_it_begins(cx: &mut TestAppContext) {
        let (list, cx) = popup(cx, vec![]);
        let mut seen = Vec::new();
        for _ in 0..4 {
            cx.simulate_keystrokes("c->c");
            seen.push(highlighted(&list, cx));
        }
        assert_eq!(seen, [Some(3), Some(4), Some(5), Some(3)]);
    }

    #[gpui::test]
    fn typeahead_skips_disabled_items(cx: &mut TestAppContext) {
        let (list, cx) = popup(cx, vec![1]);
        cx.simulate_keystrokes("b->b");
        assert_eq!(highlighted(&list, cx), Some(2));
    }
}
