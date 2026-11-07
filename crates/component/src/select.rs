use std::{cell::Cell, rc::Rc};

use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, DismissEvent, Edges, ElementId, Entity,
    EventEmitter, FocusHandle, Focusable, Hsla, InteractiveElement, IntoElement, Length,
    ParentElement, Pixels, Render, RenderOnce, SharedString, StatefulInteractiveElement,
    StyleRefinement, Styled, Window, deferred, div, point, prelude::FluentBuilder, px, rems,
};
use gpui_base::{AlignedItem, TestSupportExt as _};

use crate::ThemeStyled as _;
use crate::{
    ActiveTheme, Disableable, ElementExt as _, Icon, IconName, IndexPath, Sizable, Size,
    StyleSized, StyledExt,
    actions::Cancel,
    h_flex,
    input::{clear_button, input_style},
    list::List,
    searchable_list::{
        SearchableListChange, SearchableListDelegate, SearchableListItem, SearchableListState,
    },
    sizing::DROPDOWN_LIST_PADDING,
    v_flex,
};
use gpui_base::{GlobalState, Select as BaseSelect};

// MARK: Public re-exports for back-compat

/// Re-exported for backward compatibility. New code should prefer [`SearchableGroup`].
pub use crate::searchable_list::SearchableGroup as SelectGroup;
/// Re-exported for backward compatibility. New code should prefer [`SearchableListDelegate`].
pub use crate::searchable_list::SearchableListDelegate as SelectDelegate;
/// Re-exported for backward compatibility. New code should prefer [`SearchableListItem`].
pub use crate::searchable_list::SearchableListItem as SelectItem;
/// Re-exported for backward compatibility. New code should prefer [`SearchableListItemElement`].
pub use crate::searchable_list::SearchableListItemElement as SelectListItem;
/// Re-exported for backward compatibility.
pub use crate::searchable_list::SearchableVec;

#[derive(IntoElement)]
pub struct Caret {
    size: Size,
    color: Option<Hsla>,
}

impl Caret {
    /// Create a select caret sized for its trigger.
    pub fn new(size: Size) -> Self {
        Self { size, color: None }
    }

    /// Set the caret color.
    pub fn text_color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

impl RenderOnce for Caret {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        Icon::new(IconName::ChevronDown)
            .with_size(match self.size {
                Size::XSmall => Size::XSmall,
                Size::Small => Size::Small,
                _ => Size::Medium,
            })
            .when_some(self.color, |this, color| this.text_color(color))
    }
}

/// Events emitted by [`SelectState`].
pub enum SelectEvent<D: SearchableListDelegate + 'static>
where
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    Confirm(Option<<D::Item as SearchableListItem>::Value>),
}

// MARK: SelectOptions (builder only — applied to SearchableListState during render)

struct SelectOptions {
    style: StyleRefinement,
    size: Size,
    icon: Option<Icon>,
    cleanable: bool,
    placeholder: Option<SharedString>,
    accessibility_label: Option<SharedString>,
    title_prefix: Option<SharedString>,
    search_placeholder: Option<SharedString>,
    menu_width: Length,
    menu_max_h: Length,
    scroll_bounce: Option<bool>,
    disabled: bool,
    appearance: bool,
    focus_ring_enabled: bool,
    align_item_with_trigger: bool,
}

impl Default for SelectOptions {
    fn default() -> Self {
        Self {
            style: StyleRefinement::default(),
            size: Size::default(),
            icon: None,
            cleanable: false,
            placeholder: None,
            accessibility_label: None,
            title_prefix: None,
            menu_width: Length::Auto,
            menu_max_h: rems(20.).into(),
            scroll_bounce: None,
            disabled: false,
            appearance: true,
            focus_ring_enabled: true,
            search_placeholder: None,
            align_item_with_trigger: false,
        }
    }
}

// MARK: SelectState

/// State of the [`Select`] component.
///
/// Emits [`DismissEvent`] when an open menu closes, including after a selection is confirmed.
pub struct SelectState<D: SearchableListDelegate + 'static>
where
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    pub(crate) state: SearchableListState<D>,

    // Select-specific fields
    searchable: bool,
    icon: Option<Icon>,
    title_prefix: Option<SharedString>,
    focus_ring_enabled: bool,
    /// Whether the select asks for its popup to lie over the trigger.
    align_item_with_trigger: bool,
    /// Whether the popup open now lies over the trigger, decided as it opened
    /// so the popup does not jump when the input that follows differs.
    item_aligned: bool,
    /// Set as an item-aligned popup opens and taken by its first placement,
    /// which scrolls the list to the selected item once and then leaves the
    /// scrolling to the user.
    aligned_scroll_pending: Rc<Cell<bool>>,
}

/// A Select element.
#[derive(IntoElement)]
pub struct Select<D: SearchableListDelegate + 'static>
where
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    id: ElementId,
    state: Entity<SelectState<D>>,
    options: SelectOptions,
    empty: Option<Box<dyn Fn(&mut Window, &App) -> AnyElement + 'static>>,
}

impl<D> SelectState<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    /// Create a new Select state.
    pub fn new(
        delegate: D,
        selected_index: Option<IndexPath>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let weak = cx.entity().downgrade();
        let weak_confirm = weak.clone();
        let weak_cancel = weak.clone();
        let weak_empty = weak;

        let selected_indices = selected_index.into_iter().collect::<Vec<_>>();

        let state = SearchableListState::new(
            delegate,
            selected_indices,
            // on_confirm — commit the selection
            move |selected_index, _secondary, window, cx| {
                cx.defer_in(window, {
                    let weak_confirm = weak_confirm.clone();
                    move |list_state, window, cx| {
                        let mut selection = weak_confirm
                            .upgrade()
                            .map(|e| e.read(cx).state.selection.clone())
                            .unwrap_or_default();

                        let changes = {
                            let mut changes: Vec<SearchableListChange> = selection
                                .iter()
                                .map(|(ix, _)| SearchableListChange::Deselect { index: *ix })
                                .collect();

                            if let Some(ix) = selected_index {
                                changes.push(SearchableListChange::Select { index: ix });
                            }

                            changes
                        };

                        // on_will_change is called directly — entity-handle access would
                        // re-enter the ListState lock that defer_in holds for this callback.
                        list_state
                            .delegate_mut()
                            .delegate
                            .on_will_change(&mut selection, &changes);

                        let confirmed = weak_confirm.update(cx, |this, cx| {
                            this.state.selection = selection;

                            let final_value =
                                this.state.selection.first().map(|(_, i)| i.value().clone());

                            cx.emit(SelectEvent::Confirm(final_value.clone()));
                            cx.notify();
                            this.set_open(false, cx);
                            this.focus(window, cx);

                            (this.state.selection.clone(), final_value)
                        });

                        // Clear the query through list_state directly — an entity-handle
                        // update here would re-enter the ListState lock held above.
                        // The committed index pointed into the filtered view, so resolve
                        // the confirmed value in the restored full list; otherwise the
                        // cursor and the mark would disagree on the next open.
                        if let Ok((mut new_selection, final_value)) = confirmed {
                            if !list_state.query_input.read(cx).value().is_empty() {
                                list_state.set_query("", window, cx);
                            }

                            if let Some(ix) = final_value
                                .as_ref()
                                .and_then(|value| list_state.delegate().delegate.position(value))
                            {
                                list_state.set_selected_index(Some(ix), window, cx);
                                if let Some((slot, _)) = new_selection.first_mut() {
                                    *slot = ix;
                                }
                                _ = weak_confirm.update(cx, |this, cx| {
                                    if let Some((slot, _)) = this.state.selection.first_mut() {
                                        *slot = ix;
                                    }
                                    cx.notify();
                                });
                            }

                            list_state
                                .delegate_mut()
                                .update_selection_snapshot(new_selection.clone());
                            list_state
                                .delegate_mut()
                                .delegate
                                .on_confirm(&new_selection);
                        }
                    }
                });
            },
            // on_cancel — clear the query, restore cursor to committed index, close
            move |_final_selected_index, window, cx| {
                cx.defer_in(window, {
                    let weak_cancel = weak_cancel.clone();
                    move |list_state, window, cx| {
                        let committed_ix = weak_cancel
                            .upgrade()
                            .and_then(|e| e.read(cx).state.selection.first().map(|(ix, _)| *ix));

                        if !list_state.query_input.read(cx).value().is_empty() {
                            list_state.set_query("", window, cx);
                        }
                        list_state.set_selected_index(committed_ix, window, cx);

                        _ = weak_cancel.update(cx, |this, cx| {
                            this.set_open(false, cx);
                            this.focus(window, cx);
                        });
                    }
                });
            },
            // on_render_empty
            move |window, cx| {
                if let Some(empty) = weak_empty
                    .upgrade()
                    .and_then(|e| e.read(cx).state.empty.as_ref().map(|f| f(window, cx)))
                {
                    empty
                } else {
                    h_flex()
                        .justify_center()
                        .py_6()
                        .text_color(cx.theme().muted_foreground.opacity(0.6))
                        .child(Icon::new(IconName::Inbox).size(px(28.)))
                        .into_any_element()
                }
            },
            Self::on_blur,
            window,
            cx,
        );

        Self {
            state,
            searchable: false,
            icon: None,
            title_prefix: None,
            focus_ring_enabled: true,
            align_item_with_trigger: false,
            item_aligned: false,
            aligned_scroll_pending: Rc::new(Cell::new(false)),
        }
    }

    /// Sets whether the dropdown menu is searchable, default is `false`.
    ///
    /// When `true`, a search input appears at the top of the dropdown menu.
    pub fn searchable(mut self, searchable: bool) -> Self {
        self.searchable = searchable;
        self
    }

    /// Set the selected index for the select.
    pub fn set_selected_index(
        &mut self,
        selected_index: Option<IndexPath>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.list.update(cx, |list, cx| {
            list._set_selected_index(selected_index, window, cx);
        });

        let item = selected_index
            .and_then(|ix| self.state.list.read(cx).delegate().delegate.item(ix))
            .map(|i| i.clone());

        self.state.selection = match (selected_index, item) {
            (Some(ix), Some(item)) => vec![(ix, item)],
            _ => vec![],
        };
        self.state.sync_snapshot(cx);
    }

    /// Set selected value for the select.
    ///
    /// Looks up the position from the delegate and sets the selected index accordingly.
    /// Passes `None` when the value is not found.
    ///
    /// The delegate looks the value up in its matched items, so an active search query is
    /// cleared first to get an index into the full item list.
    pub fn set_selected_value(
        &mut self,
        selected_value: &<D::Item as SearchableListItem>::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.clear_query(window, cx);

        let selected_index = self
            .state
            .list
            .read(cx)
            .delegate()
            .delegate
            .position(selected_value);

        self.set_selected_index(selected_index, window, cx);
    }

    /// Replace the delegate (item data) for the select state.
    pub fn set_items(&mut self, items: D, window: &mut Window, cx: &mut Context<Self>)
    where
        D: SearchableListDelegate + 'static,
    {
        let mut selection = self.state.selection.clone();
        self.state.list.update(cx, |list, cx| {
            let cursor = list
                .selected_index()
                .and_then(|ix| list.delegate().delegate.item(ix))
                .and_then(|item| items.position(item.value()));
            // A value the new items no longer have is dropped, as
            // `set_selected_value` does, rather than kept at an index that
            // now points at another item.
            selection.retain_mut(|(ix, item)| {
                let Some(position) = items.position(item.value()) else {
                    return false;
                };
                let Some(replacement) = items.item(position) else {
                    return false;
                };
                *ix = position;
                *item = replacement.clone();
                true
            });
            list.delegate_mut().delegate = items;
            list.set_selected_index(cursor, window, cx);
            list.delegate_mut()
                .update_selection_snapshot(selection.clone());
            cx.notify();
        });
        self.state.selection = selection;
        cx.notify();
    }

    /// Get the current selected index.
    pub fn selected_index(&self, cx: &App) -> Option<IndexPath> {
        self.state.list.read(cx).selected_index()
    }

    /// Get the current selected value.
    pub fn selected_value(&self) -> Option<&<D::Item as SearchableListItem>::Value> {
        self.state.selection.first().map(|(_, i)| i.value())
    }

    /// Focus the select trigger input.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.state.focus_handle.focus(window, cx);
    }

    fn on_blur(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.list.read(cx).is_focused(window, cx)
            || self.state.focus_handle.is_focused(window)
        {
            return;
        }

        self.clear_query_and_restore_cursor(window, cx);
        self.set_open(false, cx);
        cx.notify();
    }

    /// Whether this select's popup asks to lie over its trigger. A searchable
    /// one never does: its search field sits where the item would have to go.
    fn aligns_item_with_trigger(&self) -> bool {
        self.align_item_with_trigger && !self.searchable
    }

    /// Records whether the popup opening now lies over the trigger.
    fn set_item_aligned(&mut self, aligned: bool) {
        self.item_aligned = aligned;
        self.aligned_scroll_pending.set(aligned);
    }

    fn toggle_menu(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();

        if self.state.open {
            self.set_open(false, cx);
            self.clear_query_and_restore_cursor(window, cx);
        } else {
            self.open_menu(window, cx);
        }

        cx.notify();
    }

    /// Opens the menu from the select's own trigger and hands it the keys.
    fn open_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_item_aligned(BaseSelect::item_alignment(
            self.aligns_item_with_trigger(),
            window,
        ));
        self.set_open(true, cx);
        self.state.list.focus_handle(cx).focus(window, cx);
    }

    fn escape(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if !self.state.open {
            cx.propagate();
            return;
        }

        cx.stop_propagation();
        self.clear_query_and_restore_cursor(window, cx);
        self.set_open(false, cx);
        self.focus(window, cx);
        cx.notify();
    }

    /// Drop the search query and move the cursor back to the committed
    /// selection, so the next open shows every item. Call on every menu
    /// close that does not go through the confirm/cancel callbacks.
    fn clear_query_and_restore_cursor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.clear_query(window, cx);

        let committed_ix = self.state.selection.first().map(|(ix, _)| *ix);
        self.state.list.update(cx, |list, cx| {
            if list.selected_index() != committed_ix {
                list.set_selected_index(committed_ix, window, cx);
            }
        });
    }

    /// Typing into a closed select selects the item the typed text begins,
    /// as a native pop-up button does, without opening the menu.
    fn typeahead(&mut self, character: char, window: &mut Window, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let Some(ix) = self
            .state
            .list
            .update(cx, |list, cx| list.typeahead(character, now, cx))
        else {
            return;
        };

        let previous = self.selected_value().cloned();
        let mut selection = self.state.selection.clone();
        let changes: Vec<SearchableListChange> = selection
            .iter()
            .map(|(ix, _)| SearchableListChange::Deselect { index: *ix })
            .chain([SearchableListChange::Select { index: ix }])
            .collect();
        self.state.list.update(cx, |list, _| {
            list.delegate_mut()
                .delegate
                .on_will_change(&mut selection, &changes)
        });
        self.state.selection = selection;

        // A delegate that vetoed the change leaves the cursor on what it kept.
        let committed_ix = self.state.selection.first().map(|(ix, _)| *ix);
        let committed = self.state.selection.clone();
        self.state.list.update(cx, |list, cx| {
            list.set_selected_index(committed_ix, window, cx);
            list.delegate_mut()
                .update_selection_snapshot(committed.clone());
            list.delegate_mut().delegate.on_confirm(&committed);
            cx.notify();
        });

        let value = self.selected_value().cloned();
        if value != previous {
            cx.emit(SelectEvent::Confirm(value));
        }
        cx.notify();
    }

    fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        let dismissed = self.state.open && !open;
        self.state.open = open;
        self.state.deferred_context = open.then(|| GlobalState::register_deferred_popover(cx));

        if dismissed {
            cx.emit(DismissEvent);
        }
        cx.notify();
    }

    fn clean(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.set_selected_index(None, window, cx);
        cx.emit(SelectEvent::Confirm(None));
    }

    fn display_title(&mut self, _: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let default_title = div().text_color(cx.theme().muted_foreground).child(
            self.state
                .placeholder
                .clone()
                .unwrap_or_else(|| "Please select".into()),
        );

        // The committed value, not the highlight, which follows the keys and
        // the pointer while the menu is open.
        let Some(title) = self.state.selection.first().map(|(_, item)| {
            if let Some(el) = item.display_title() {
                el
            } else if let Some(prefix) = self.title_prefix.as_ref() {
                format!("{}{}", prefix, item.title()).into_any_element()
            } else {
                item.title().into_any_element()
            }
        }) else {
            return default_title;
        };

        div()
            .when(self.state.disabled, |this| {
                this.text_color(cx.theme().muted_foreground)
            })
            .child(title)
    }

    fn accessibility_value(&self) -> SharedString {
        let Some((_, item)) = self.state.selection.first() else {
            return self
                .state
                .placeholder
                .clone()
                .unwrap_or_else(|| "Please select".into());
        };

        if let Some(prefix) = self.title_prefix.as_ref() {
            format!("{}{}", prefix, item.title()).into()
        } else {
            item.title()
        }
    }
}

impl<D> Render for SelectState<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let searchable = self.searchable;
        let is_focused = self.state.focus_handle.is_focused(window);
        let show_clean = self.state.cleanable && !self.state.selection.is_empty();
        let bounds = self.state.bounds;
        let allow_open = !(self.state.open || self.state.disabled);
        let outline_visible = self.state.open || (is_focused && !self.state.disabled);

        let (bg, fg) = input_style(self.state.disabled, cx);

        self.state.list.update(cx, |list, cx| {
            list.set_searchable(searchable, cx);
            list.delegate_mut().size = self.state.size;
        });
        let popup = self.render_popup(bounds, window, cx);

        div().size_full().relative().child(
            div()
                .relative()
                .on_prepaint({
                    let state = cx.entity();
                    move |bounds, _, cx| state.update(cx, |r, _| r.state.bounds = bounds)
                })
                .child(
                    div()
                        .id("input")
                        .test_support()
                        .relative()
                        .flex()
                        .items_center()
                        .justify_between()
                        .border_1()
                        .border_color(cx.theme().transparent)
                        .when(self.state.appearance, |this| {
                            this.bg(bg)
                                .text_color(fg)
                                .when(self.state.disabled, |this| this.opacity(0.5))
                                .border_color(cx.theme().input)
                                .rounded(cx.theme().radius)
                        })
                        .input_size(self.state.size)
                        .input_text_size(self.state.size)
                        .refine_style(&self.state.style)
                        .when(outline_visible && self.state.appearance, |this| {
                            this.border_1().border_color(cx.theme().ring)
                        })
                        .when(
                            outline_visible && self.state.appearance && self.focus_ring_enabled,
                            |this| this.focus_ring_style(window, cx),
                        )
                        .when(allow_open, |this| {
                            this.on_click(cx.listener(Self::toggle_menu))
                        })
                        .child(
                            h_flex()
                                .id("inner")
                                .w_full()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .items_center()
                                .justify_between()
                                .gap_1()
                                .child(
                                    div()
                                        .id("title")
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .truncate()
                                        .child(self.display_title(window, cx)),
                                )
                                .when(show_clean, |this| {
                                    this.child(clear_button(cx).map(|this| {
                                        if self.state.disabled {
                                            this.disabled(true)
                                        } else {
                                            this.on_click(cx.listener(Self::clean))
                                        }
                                    }))
                                })
                                .when(!show_clean, |this| {
                                    let icon = match self.icon.clone() {
                                        Some(icon) => icon
                                            .xsmall()
                                            .text_color(cx.theme().muted_foreground)
                                            .into_any_element(),
                                        None => Caret::new(self.state.size)
                                            .text_color(cx.theme().muted_foreground)
                                            .into_any_element(),
                                    };

                                    this.child(icon)
                                }),
                        ),
                )
                .children(popup),
        )
    }
}

impl<D> SelectState<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    /// The list under the trigger, drawn through the dropdown's motion in
    /// and out (see [`crate::popover::dropdown_popup`]); nothing once it is
    /// closed and settled.
    fn render_popup(
        &self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<D>> {
        let open = self.state.open;
        let surface = v_flex()
            .occlude()
            .map(|this| match self.state.menu_width {
                Length::Auto => this.w(bounds.size.width),
                Length::Definite(w) => this.w(w),
            })
            .popover_style(cx)
            .child(
                List::new(&self.state.list)
                    .when_some(
                        self.state.search_placeholder.clone(),
                        |this, placeholder| this.search_placeholder(placeholder),
                    )
                    .with_size(self.state.size)
                    .max_h(self.state.menu_max_h)
                    .when_some(self.state.scroll_bounce, |list, on| list.scroll_bounce(on))
                    .paddings(Edges::all(DROPDOWN_LIST_PADDING)),
            )
            .when(open, |this| {
                this.on_mouse_down_out(cx.listener(|this, _, window, cx| {
                    this.escape(&Cancel, window, cx);
                }))
            });
        let positioner = crate::popover::dropdown_positioner(bounds);
        let positioner = if self.item_aligned {
            self.align_item_with_trigger(positioner, cx)
        } else {
            positioner
        };
        crate::popover::dropdown_popup(
            ("select-popup", cx.entity_id()),
            open,
            positioner,
            surface,
            window,
            cx,
        )
        .map(|popup| deferred(popup).with_priority(gpui_base::POPUP_PRIORITY))
    }

    /// Places the popup so its selected item lies over the trigger, as Base
    /// UI's `alignItemWithTrigger` does. An aligned popup has no side to
    /// slide out of, so it only fades, as Base UI's does; with no item
    /// selected, or no room to align it, the popup falls back to the
    /// dropdown's place below or above the trigger and slides out of that
    /// edge as the dropdown does.
    fn align_item_with_trigger(
        &self,
        positioner: gpui_base::Positioner,
        cx: &App,
    ) -> gpui_base::Positioner {
        let list = self.state.list.clone();
        let selected = self.state.selection.first().map(|(ix, _)| *ix);
        let scroll_handle = list.read(cx).scroll_handle().clone();
        let scroll_pending = self.aligned_scroll_pending.clone();

        positioner
            .align_item_with_trigger(move |cx| {
                // The row's place among the rows, inside the list's padding;
                // the popup draws no border, so its edge is the list's.
                let (top, height, rows) = list.read(cx).item_extent(selected?)?;
                Some(
                    AlignedItem::new(DROPDOWN_LIST_PADDING + top, height)
                        .with_content_height(rows + DROPDOWN_LIST_PADDING * 2.),
                )
            })
            .on_position(move |position| {
                if let Some(scroll) = position.item_scroll
                    && scroll_pending.replace(false)
                {
                    scroll_handle.set_offset(point(scroll_handle.offset().x, -scroll));
                }
            })
    }
}

impl<D> Select<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    pub fn new(state: &Entity<SelectState<D>>) -> Self {
        Self {
            id: ("select", state.entity_id()).into(),
            state: state.clone(),
            options: SelectOptions::default(),
            empty: None,
        }
    }

    /// Sets an explicit identity for the select root.
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = id.into();
        self
    }

    /// Set the width of the dropdown menu, default: `Length::Auto`.
    pub fn menu_width(mut self, width: impl Into<Length>) -> Self {
        self.options.menu_width = width.into();
        self
    }

    /// Set the max height of the dropdown menu, default: 20rem.
    pub fn menu_max_h(mut self, max_h: impl Into<Length>) -> Self {
        self.options.menu_max_h = max_h.into();
        self
    }

    /// Set the placeholder shown when no value is selected.
    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.options.placeholder = Some(placeholder.into());
        self
    }

    /// Set the name a screen reader announces for the select.
    ///
    /// The placeholder and selected value are not used as the accessible name,
    /// because they describe the current value rather than the control itself.
    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.options.accessibility_label = Some(label.into());
        self
    }

    /// Override the trailing icon, replacing the default chevron.
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.options.icon = Some(icon.into());
        self
    }

    /// Set a label prefix shown before the selected title in the trigger.
    ///
    /// e.g. `title_prefix("Country: ")` → "Country: United States"
    pub fn title_prefix(mut self, prefix: impl Into<SharedString>) -> Self {
        self.options.title_prefix = Some(prefix.into());
        self
    }

    /// Show a clear button when a value is selected.
    pub fn cleanable(mut self, cleanable: bool) -> Self {
        self.options.cleanable = cleanable;
        self
    }

    /// Set the placeholder text for the search input.
    pub fn search_placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.options.search_placeholder = Some(placeholder.into());
        self
    }

    /// Bounce the dropdown list at its vertical boundaries. On by default;
    /// reduced motion disables displacement.
    pub fn scroll_bounce(mut self, enabled: bool) -> Self {
        self.options.scroll_bounce = Some(enabled);
        self
    }

    /// Set the disabled state.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.options.disabled = disabled;
        self
    }

    /// Set a custom closure that renders the empty-state element.
    pub fn empty<E: IntoElement + 'static>(
        mut self,
        builder: impl Fn(&mut Window, &App) -> E + 'static,
    ) -> Self {
        self.empty = Some(Box::new(move |window, cx| {
            builder(window, cx).into_any_element()
        }));
        self
    }

    /// Control whether the trigger shows a border and background (`true` by default).
    pub fn appearance(mut self, appearance: bool) -> Self {
        self.options.appearance = appearance;
        self
    }

    /// Lay the open list over the trigger with the selected item exactly
    /// where the trigger shows it, as Base UI's `alignItemWithTrigger` does,
    /// instead of hanging the list below the trigger. Off by default.
    ///
    /// The list scrolls as far as that takes and stays inside the window.
    /// It still opens below or above the trigger when nothing is selected,
    /// when there is no room to align the item, when the select is opened by
    /// touch, and when it is searchable, whose search field sits where the
    /// item would have to go.
    pub fn align_item_with_trigger(mut self, align: bool) -> Self {
        self.options.align_item_with_trigger = align;
        self
    }
}

impl<D> Sizable for Select<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.options.size = size.into();
        self
    }
}

impl<D> crate::FocusableExt for Select<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    fn focus_ring(mut self, enabled: bool) -> Self {
        self.options.focus_ring_enabled = enabled;
        self
    }

    fn is_focus_ring_enabled(&self) -> bool {
        self.options.focus_ring_enabled
    }
}

impl<D> EventEmitter<SelectEvent<D>> for SelectState<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
}

impl<D> EventEmitter<DismissEvent> for SelectState<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
}

impl<D> Focusable for SelectState<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.state.open {
            self.state.list.focus_handle(cx)
        } else {
            self.state.focus_handle.clone()
        }
    }
}

impl<D> Styled for Select<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.options.style
    }
}

impl<D> RenderOnce for Select<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let disabled = self.options.disabled;
        let accessibility_label = self.options.accessibility_label.clone();
        let focus_handle = self.state.read(cx).state.focus_handle.clone();
        let empty = self.empty;
        let opts = self.options;

        self.state.update(cx, |this, _| {
            this.state.style = opts.style;
            this.state.size = opts.size;
            this.state.cleanable = opts.cleanable;
            this.state.placeholder = opts.placeholder;
            this.state.search_placeholder = opts.search_placeholder;
            this.state.menu_width = opts.menu_width;
            this.state.menu_max_h = opts.menu_max_h;
            this.state.scroll_bounce = opts.scroll_bounce;
            this.state.disabled = opts.disabled;
            this.state.appearance = opts.appearance;
            this.focus_ring_enabled = opts.focus_ring_enabled;
            this.icon = opts.icon;
            this.title_prefix = opts.title_prefix;
            this.align_item_with_trigger = opts.align_item_with_trigger;

            if let Some(empty) = empty {
                this.state.empty = Some(empty);
            }
        });

        let is_open = self.state.read(cx).state.open;
        let accessibility_value = self.state.read(cx).accessibility_value();
        let content_focus_handle = self.state.read(cx).state.list.focus_handle(cx);
        let searchable = self.state.read(cx).searchable;
        let aligns_item_with_trigger = self.state.read(cx).aligns_item_with_trigger();
        let open_state = self.state.clone();
        let typeahead_state = self.state.clone();
        let alignment_state = self.state.clone();

        BaseSelect::new(self.id)
            .open(is_open)
            .disabled(disabled)
            .when_some(accessibility_label, |this, label| {
                this.accessibility_label(label)
            })
            .focus_handle(&focus_handle)
            .content_focus_handle(&content_focus_handle)
            .accessibility_value(accessibility_value)
            .align_item_with_trigger(aligns_item_with_trigger)
            .on_item_alignment(move |aligned, _, cx| {
                alignment_state.update(cx, |state, _| state.set_item_aligned(aligned));
            })
            .on_open_change(move |open, window, cx| {
                open_state.update(cx, |state, cx| {
                    if !open {
                        state.clear_query_and_restore_cursor(window, cx);
                    }
                    state.set_open(open, cx);
                });
            })
            // A searchable select types into its search field instead.
            .when(!searchable, |this| {
                this.on_typeahead(move |character, window, cx| {
                    typeahead_state.update(cx, |state, cx| state.typeahead(character, window, cx));
                })
            })
            .size_full()
            .child(self.state)
    }
}

// MARK: Tests

#[cfg(test)]
mod tests {
    use gpui::{AppContext as _, RenderOnce as _, TestAppContext};

    use crate::{
        IndexPath,
        searchable_list::{SearchableListDelegate as _, SearchableVec},
        select::{Select, SelectGroup, SelectState},
    };

    #[gpui::test]
    fn an_explicit_accessibility_label_does_not_replace_the_placeholder(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let items = SearchableVec::new(vec!["Rust", "Go", "C++"]);
            let state = cx.new(|cx| SelectState::new(items, None, window, cx));

            let plain = Select::new(&state).placeholder("Choose a language");
            assert_eq!(plain.options.accessibility_label, None);
            assert_eq!(
                plain.options.placeholder.as_deref(),
                Some("Choose a language")
            );

            let named = Select::new(&state)
                .placeholder("Choose a language")
                .accessibility_label("Programming language");
            assert_eq!(
                named.options.accessibility_label.as_deref(),
                Some("Programming language")
            );
            assert_eq!(
                named.options.placeholder.as_deref(),
                Some("Choose a language"),
                "an accessible name must not change what is drawn"
            );
        });
    }

    #[gpui::test]
    fn test_select_initial_selection_seeds_cursor(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let items = SearchableVec::new(vec!["Rust", "Go", "C++"]);
            let state = cx.new(|cx| SelectState::new(items, Some(IndexPath::new(1)), window, cx));

            assert_eq!(
                state.read(cx).selected_index(cx),
                Some(IndexPath::new(1)),
                "initial cursor should be seeded on ListState so display_title can read it",
            );
            assert_eq!(state.read(cx).selected_value(), Some(&"Go"));
        });
    }

    #[gpui::test]
    fn test_select_initial_grouped_selection_seeds_cursor(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let mut groups: SearchableVec<SelectGroup<&'static str>> = SearchableVec::new(vec![]);
            groups.push(SelectGroup::new("A").items(["Apple", "Avocado"]));
            groups.push(SelectGroup::new("B").items(["Banana", "Blueberry", "Blackberry"]));

            let initial = IndexPath::new(1).section(1);
            let state = cx.new(|cx| SelectState::new(groups, Some(initial), window, cx));

            assert_eq!(state.read(cx).selected_index(cx), Some(initial));
            assert_eq!(state.read(cx).selected_value(), Some(&"Blueberry"));
        });
    }

    #[gpui::test]
    fn test_select_set_selected_value_clears_search_query(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let items = SearchableVec::new(vec!["Rust", "Go", "C++"]);
            let state = cx.new(|cx| SelectState::new(items, None, window, cx).searchable(true));
            let list = state.read(cx).state.list.clone();

            list.update(cx, |list, cx| list.set_query("Rust", window, cx));
            assert_eq!(list.read(cx).delegate().delegate.items_count(0), 1);

            state.update(cx, |state, cx| {
                state.set_selected_value(&"Go", window, cx);
            });

            assert_eq!(state.read(cx).selected_value(), Some(&"Go"));
            assert_eq!(state.read(cx).selected_index(cx), Some(IndexPath::new(1)));
            assert_eq!(list.read(cx).query_input.read(cx).value(), "");
        });
    }

    #[gpui::test]
    fn test_select_set_selected_value_clears_grouped_search_query(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let mut groups: SearchableVec<SelectGroup<&'static str>> = SearchableVec::new(vec![]);
            groups.push(SelectGroup::new("A").items(["Apple", "Avocado"]));
            groups.push(SelectGroup::new("B").items(["Banana", "Blueberry"]));

            let state = cx.new(|cx| SelectState::new(groups, None, window, cx).searchable(true));
            let list = state.read(cx).state.list.clone();

            list.update(cx, |list, cx| list.set_query("Blue", window, cx));
            state.update(cx, |state, cx| {
                state.set_selected_value(&"Banana", window, cx);
            });

            assert_eq!(state.read(cx).selected_value(), Some(&"Banana"));
            assert_eq!(
                state.read(cx).selected_index(cx),
                Some(IndexPath::new(0).section(1)),
            );
        });
    }

    #[gpui::test]
    fn test_select_accessibility_value_tracks_placeholder_and_selection(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let window = cx.add_empty_window();
        window.update(|window, cx| {
            let items = SearchableVec::new(vec!["Rust", "Go"]);
            let state = cx.new(|cx| SelectState::new(items, None, window, cx).searchable(true));

            _ = Select::new(&state)
                .placeholder("Choose a language")
                .accessibility_label("Programming language")
                .render(window, cx);
            assert_eq!(state.read(cx).accessibility_value(), "Choose a language");

            state.update(cx, |state, cx| {
                state.set_selected_value(&"Rust", window, cx);
            });
            assert_eq!(state.read(cx).accessibility_value(), "Rust");

            let list = state.read(cx).state.list.clone();
            list.update(cx, |list, cx| list.set_query("Go", window, cx));
            assert_eq!(list.read(cx).delegate().delegate.items_count(0), 1);
            // Filtering changes the available rows, not the committed value.
            assert_eq!(state.read(cx).accessibility_value(), "Rust");

            _ = Select::new(&state)
                .placeholder("Choose a language")
                .title_prefix("Language: ")
                .render(window, cx);
            assert_eq!(state.read(cx).accessibility_value(), "Language: Rust");

            state.update(cx, |state, cx| state.set_selected_index(None, window, cx));
            assert_eq!(state.read(cx).accessibility_value(), "Choose a language");

            _ = Select::new(&state).render(window, cx);
            assert_eq!(
                state.read(cx).accessibility_value(),
                "Please select".to_string(),
            );
        });
    }

    #[gpui::test]
    fn replacing_items_reconciles_the_cursor_and_selection(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let state = cx.new(|cx| {
                SelectState::new(
                    vec!["main", "topic", "release"],
                    Some(IndexPath::new(0)),
                    window,
                    cx,
                )
            });
            let list = state.read(cx).state.list.clone();
            list.update(cx, |list, cx| {
                list.set_selected_index(Some(IndexPath::new(1)), window, cx)
            });
            state.update(cx, |state, cx| {
                state.set_items(vec!["release", "main", "topic"], window, cx)
            });
            assert_eq!(state.read(cx).selected_index(cx), Some(IndexPath::new(2)));
            assert_eq!(state.read(cx).selected_value(), Some(&"main"));
            assert_eq!(state.read(cx).state.selection[0].0, IndexPath::new(1));
            assert_eq!(
                list.read(cx).delegate().selection_snapshot[0].0,
                IndexPath::new(1)
            );
            state.update(cx, |state, cx| {
                state.set_items(vec!["main", "replacement"], window, cx)
            });
            assert_eq!(state.read(cx).selected_index(cx), None);
            assert_eq!(state.read(cx).selected_value(), Some(&"main"));

            // A selected value the new items no longer have is dropped.
            state.update(cx, |state, cx| {
                state.set_items(vec!["replacement", "other"], window, cx)
            });
            assert_eq!(state.read(cx).selected_value(), None);
            assert!(state.read(cx).state.selection.is_empty());
            assert!(list.read(cx).delegate().selection_snapshot.is_empty());
        });
    }

    #[derive(Clone)]
    struct RemoteChoice {
        value: &'static str,
        title: &'static str,
        unavailable: bool,
    }

    impl super::SelectItem for RemoteChoice {
        type Value = &'static str;
        fn value(&self) -> &&'static str {
            &self.value
        }
        fn title(&self) -> gpui::SharedString {
            self.title.into()
        }
        fn disabled(&self) -> bool {
            self.unavailable
        }
    }

    #[gpui::test]
    fn disabled_items_cannot_be_confirmed(cx: &mut TestAppContext) {
        use crate::list::ListDelegate as _;
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        for unavailable in [true, false] {
            let state = cx.update(|window, cx| {
                let state = cx.new(|cx| {
                    SelectState::new(
                        vec![RemoteChoice {
                            value: "main",
                            title: "Main",
                            unavailable,
                        }],
                        Some(IndexPath::new(0)),
                        window,
                        cx,
                    )
                });
                state.update(cx, |state, cx| state.set_open(true, cx));
                let list = state.read(cx).state.list.clone();
                list.update(cx, |list, cx| {
                    list.delegate_mut().confirm(false, window, cx)
                });
                state
            });
            cx.run_until_parked();
            cx.update(|_, cx| assert_eq!(state.read(cx).state.open, unavailable));
        }
    }

    /// Typeahead, as a native pop-up button and Base UI's Select have it.
    mod typeahead {
        use std::{cell::RefCell, rc::Rc};

        use gpui::{
            AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
            TestAppContext, VisualTestContext, Window, div, px,
        };

        use crate::{
            IndexPath,
            searchable_list::SearchableVec,
            select::{Select, SelectEvent, SelectState},
        };

        type Fruits = SearchableVec<&'static str>;
        type Confirmed = Rc<RefCell<Vec<Option<&'static str>>>>;

        struct Form {
            state: Entity<SelectState<Fruits>>,
        }

        impl Render for Form {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().w(px(240.)).child(Select::new(&self.state))
            }
        }

        /// A focused, closed select of fruits with `Apple` selected, and the
        /// values it confirms.
        fn form(
            cx: &mut TestAppContext,
            searchable: bool,
        ) -> (
            Entity<SelectState<Fruits>>,
            Confirmed,
            &mut VisualTestContext,
        ) {
            cx.update(crate::init);
            let (form, cx) = cx.add_window_view(|window, cx| {
                let items = SearchableVec::new(vec!["Apple", "Banana", "Blueberry", "Cherry"]);
                Form {
                    state: cx.new(|cx| {
                        SelectState::new(items, Some(IndexPath::new(0)), window, cx)
                            .searchable(searchable)
                    }),
                }
            });
            let state = form.read_with(cx, |form, _| form.state.clone());
            let confirmed = Confirmed::default();
            cx.update(|window, cx| {
                let confirmed = confirmed.clone();
                cx.subscribe(&state, move |_, event: &SelectEvent<Fruits>, _| {
                    let SelectEvent::Confirm(value) = event;
                    confirmed.borrow_mut().push(*value);
                })
                .detach();
                state.update(cx, |state, cx| state.focus(window, cx));
                window.draw(cx).clear(cx);
            });
            (state, confirmed, cx)
        }

        fn selected(
            state: &Entity<SelectState<Fruits>>,
            cx: &mut VisualTestContext,
        ) -> Option<&'static str> {
            state.read_with(cx, |state, _| state.selected_value().copied())
        }

        #[gpui::test]
        fn typing_into_a_closed_select_selects_the_item_it_begins(cx: &mut TestAppContext) {
            let (state, confirmed, cx) = form(cx, false);
            cx.simulate_keystrokes("b->b");
            assert_eq!(selected(&state, cx), Some("Banana"));
            cx.simulate_keystrokes("b->b");
            assert_eq!(
                selected(&state, cx),
                Some("Blueberry"),
                "a repeated `b` moves on"
            );
            assert!(
                !state.read_with(cx, |state, _| state.state.open),
                "the menu stays closed"
            );
            assert_eq!(*confirmed.borrow(), [Some("Banana"), Some("Blueberry")]);
        }

        #[gpui::test]
        fn typing_into_an_open_select_highlights_and_enter_commits(cx: &mut TestAppContext) {
            let (state, confirmed, cx) = form(cx, false);
            cx.simulate_keystrokes("space");
            assert!(state.read_with(cx, |state, _| state.state.open));
            cx.update(|window, cx| window.draw(cx).clear(cx));

            cx.simulate_keystrokes("c->c");
            assert_eq!(
                state.read_with(cx, |state, cx| state.selected_index(cx)),
                Some(IndexPath::new(3)),
                "`Cherry` is highlighted"
            );
            assert_eq!(selected(&state, cx), Some("Apple"), "but not yet selected");
            assert!(confirmed.borrow().is_empty());

            cx.simulate_keystrokes("enter");
            assert_eq!(selected(&state, cx), Some("Cherry"));
            assert!(!state.read_with(cx, |state, _| state.state.open));
        }

        #[gpui::test]
        fn a_closed_searchable_select_ignores_typing(cx: &mut TestAppContext) {
            let (state, confirmed, cx) = form(cx, true);
            cx.simulate_keystrokes("b->b");
            assert_eq!(selected(&state, cx), Some("Apple"));
            assert!(confirmed.borrow().is_empty());
        }
    }

    /// The list laid over the trigger, as Base UI's `alignItemWithTrigger`
    /// does.
    mod item_alignment {
        use std::time::Duration;

        use gpui::{
            AppContext as _, Bounds, Context, Entity, IntoElement, Modifiers, ParentElement as _,
            Pixels, Render, Styled as _, TestAppContext, TouchEvent, TouchPhase, VisualTestContext,
            Window, div, px, size,
        };

        use crate::{
            IndexPath,
            searchable_list::SearchableVec,
            select::{Select, SelectState},
        };

        type Numbers = SearchableVec<&'static str>;

        struct Form {
            state: Entity<SelectState<Numbers>>,
            align: bool,
            top: Pixels,
        }

        impl Render for Form {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().size_full().child(
                    div()
                        .absolute()
                        .top(self.top)
                        .left(px(40.))
                        .w(px(200.))
                        .child(Select::new(&self.state).align_item_with_trigger(self.align)),
                )
            }
        }

        /// A select of eight numbers with `selected` chosen, its trigger
        /// `top` down a 400 by 600 window.
        fn form(
            cx: &mut TestAppContext,
            align: bool,
            top: f32,
            selected: usize,
        ) -> (Entity<SelectState<Numbers>>, &mut VisualTestContext) {
            cx.update(crate::init);
            let (form, cx) = cx.add_window_view(|window, cx| {
                let items = SearchableVec::new(vec![
                    "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight",
                ]);
                Form {
                    state: cx.new(|cx| {
                        SelectState::new(items, Some(IndexPath::new(selected)), window, cx)
                    }),
                    align,
                    top: px(top),
                }
            });
            cx.simulate_resize(size(px(400.), px(600.)));
            let state = form.read_with(cx, |form, _| form.state.clone());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            (state, cx)
        }

        fn trigger(
            state: &Entity<SelectState<Numbers>>,
            cx: &mut VisualTestContext,
        ) -> Bounds<Pixels> {
            state.read_with(cx, |state, _| state.state.bounds)
        }

        /// Lets the open motion finish and draws the settled popup.
        fn settle(cx: &mut VisualTestContext) {
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.executor().advance_clock(Duration::from_secs(1));
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }

        #[gpui::test]
        fn the_selected_item_opens_over_the_trigger(cx: &mut TestAppContext) {
            let (state, cx) = form(cx, true, 300., 4);
            let trigger = trigger(&state, cx);
            cx.simulate_click(trigger.center(), Modifiers::none());
            settle(cx);

            assert!(state.read_with(cx, |state, _| state.state.open));
            let row = cx.debug_bounds("list-item-0-4").expect("`Five` is drawn");
            assert!(
                (row.center().y - trigger.center().y).abs() <= px(1.),
                "`Five` sits over the trigger: {row:?} against {trigger:?}"
            );
        }

        #[gpui::test]
        fn without_it_the_list_hangs_below_the_trigger(cx: &mut TestAppContext) {
            let (state, cx) = form(cx, false, 300., 4);
            let trigger = trigger(&state, cx);
            cx.simulate_click(trigger.center(), Modifiers::none());
            settle(cx);

            let first = cx.debug_bounds("list-item-0-0").expect("`One` is drawn");
            assert!(
                first.top() >= trigger.bottom(),
                "{first:?} under {trigger:?}"
            );
        }

        /// A trigger near the top of the window leaves no room for `Seven`
        /// to rise over it, so the list opens below as usual.
        #[gpui::test]
        fn an_item_with_no_room_to_align_opens_below(cx: &mut TestAppContext) {
            let (state, cx) = form(cx, true, 10., 6);
            let trigger = trigger(&state, cx);
            cx.simulate_click(trigger.center(), Modifiers::none());
            settle(cx);

            let first = cx.debug_bounds("list-item-0-0").expect("`One` is drawn");
            assert!(
                first.top() >= trigger.bottom(),
                "{first:?} under {trigger:?}"
            );
        }

        /// Base UI does not align a select opened by touch.
        #[gpui::test]
        fn a_select_opened_by_touch_opens_below(cx: &mut TestAppContext) {
            let (state, cx) = form(cx, true, 300., 4);
            let trigger = trigger(&state, cx);
            cx.simulate_event(TouchEvent {
                phase: TouchPhase::Started,
                position: trigger.center(),
                ..Default::default()
            });
            cx.update(|window, cx| state.update(cx, |state, cx| state.open_menu(window, cx)));
            settle(cx);

            let first = cx.debug_bounds("list-item-0-0").expect("`One` is drawn");
            assert!(
                first.top() >= trigger.bottom(),
                "{first:?} under {trigger:?}"
            );
        }
    }

    /// A page holding a select under retained views.
    mod retained {
        use std::{cell::Cell, rc::Rc};

        use gpui::{
            AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
            TestAppContext, Window, div, px,
        };

        use crate::{
            searchable_list::SearchableVec,
            select::{Select, SelectState},
        };

        /// A page that shows a select and what it selected, counting its
        /// renders.
        struct Page {
            state: Entity<SelectState<SearchableVec<&'static str>>>,
            renders: Rc<Cell<usize>>,
        }

        impl Render for Page {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                self.renders.set(self.renders.get() + 1);
                let selected = self.state.read(cx).selected_index(cx);
                div()
                    .w(px(240.))
                    .child(Select::new(&self.state))
                    .child(format!("{selected:?}"))
            }
        }

        /// Drawing a select again changes nothing in it: the page around it,
        /// which reads its selection, is not built again for it.
        #[gpui::test]
        fn drawing_a_select_again_leaves_the_page_around_it_retained(cx: &mut TestAppContext) {
            cx.update(crate::init);
            let renders = Rc::new(Cell::new(0));
            let (_, cx) = cx.add_window_view({
                let renders = renders.clone();
                move |window, cx| {
                    let items = SearchableVec::new(vec!["Rust", "Go", "C++"]);
                    Page {
                        state: cx.new(|cx| SelectState::new(items, None, window, cx)),
                        renders,
                    }
                }
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let rendered = renders.get();
            for _ in 0..3 {
                cx.update(|window, cx| window.draw(cx).clear(cx));
            }
            assert_eq!(renders.get(), rendered, "the page was built again");
        }
    }
}
