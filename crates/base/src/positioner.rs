//! Shared popup positioning.
//!
//! Every anchored surface in Base resolves its position here so that flipping,
//! alignment, and viewport clamping cannot drift apart between popups,
//! tooltips, and menus.

use gpui::{
    Anchor, AnyElement, App, Bounds, Decorations, Display, Edges, Element, GlobalElementId,
    Half as _, HitboxBehavior, InspectorElementId, IntoElement, LayoutId, ParentElement, Pixels,
    Point, Position, Size, Style, Window, point, px,
};

use crate::Placement;
use std::{cell::Cell, rc::Rc};

/// Alignment of a popup along the side it is placed on.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Align {
    /// Align with the trigger's leading edge.
    Start,
    /// Center on the trigger.
    #[default]
    Center,
    /// Align with the trigger's trailing edge.
    End,
}

/// How a popup derives its position.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Strategy {
    /// Place `anchor`'s corner of the popup at `position`, then clamp into the
    /// viewport. With `around`, the trigger the position was taken from, a
    /// popup that does not fit on its side of the trigger flips to the other
    /// side when that has more room, as Base UI's positioner does.
    Corner {
        anchor: Anchor,
        position: Point<Pixels>,
        around: Option<Bounds<Pixels>>,
    },
    /// Place the popup on `placement`'s side of the trigger, flipping to the
    /// opposite side when it does not fit, then clamp into the viewport.
    Side {
        trigger_bounds: Bounds<Pixels>,
        placement: Option<Placement>,
        align: Align,
        offset: Pixels,
    },
}

/// The bounds a popup resolved to, and the side it ended up on.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ResolvedPosition {
    /// Final popup bounds in window coordinates.
    pub bounds: Bounds<Pixels>,
    /// The side the popup was placed on. `None` for corner positioning, which
    /// has no notion of a side.
    pub placement: Option<Placement>,
    /// The corner a corner-positioned popup was placed by: the one asked for,
    /// or its mirror when it flipped around its trigger. `None` for side
    /// positioning.
    pub anchor: Option<Anchor>,
    /// How far the popup's list must be scrolled for the item
    /// [`Positioner::align_item_with_trigger`] names to sit over the trigger.
    /// `None` when the popup was placed on a side or at a corner instead.
    pub item_scroll: Option<Pixels>,
}

/// Where the item an item-aligned popup lines up with its trigger sits in the
/// popup, measured from the popup's top edge with its list scrolled to the top.
///
/// See [`Positioner::align_item_with_trigger`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AlignedItem {
    top: Pixels,
    height: Pixels,
    content_height: Pixels,
}

impl AlignedItem {
    /// An item whose top edge is `top` below the popup's top edge and that is
    /// `height` tall.
    pub fn new(top: Pixels, height: Pixels) -> Self {
        Self {
            top,
            height,
            content_height: px(0.),
        }
    }

    /// Sets the height the popup would have if nothing in it were clipped.
    ///
    /// The difference from the height it is laid out at is how far its list
    /// can scroll. Without it the list is taken not to scroll at all.
    pub fn with_content_height(mut self, content_height: Pixels) -> Self {
        self.content_height = content_height;
        self
    }

    /// The item's top edge, below the popup's top edge, with the list
    /// unscrolled.
    pub fn top(&self) -> Pixels {
        self.top
    }

    /// The item's height.
    pub fn height(&self) -> Pixels {
        self.height
    }

    /// The height the popup would have if nothing in it were clipped.
    pub fn content_height(&self) -> Pixels {
        self.content_height
    }
}

type AlignedItemSource = Box<dyn Fn(&App) -> Option<AlignedItem>>;

/// An unstyled element that positions its children as an anchored popup.
///
/// This owns measurement, side selection, alignment, and viewport clamping. It
/// installs no presentation of its own and adds no wrapper element around its
/// children.
pub struct Positioner {
    strategy: Strategy,
    corner_position: Option<Rc<Cell<Point<Pixels>>>>,
    corner_around: Option<Rc<Cell<Bounds<Pixels>>>>,
    aligned_item: Option<AlignedItemSource>,
    on_position: Option<Box<dyn Fn(ResolvedPosition)>>,
    margin: Pixels,
    approach: Pixels,
    occlude: bool,
    children: Vec<AnyElement>,
}

#[doc(hidden)]
pub struct PositionerState {
    child_layout_ids: Vec<LayoutId>,
}

impl Positioner {
    /// Places the popup on a side of `trigger_bounds`, flipping when needed.
    pub fn side(trigger_bounds: Bounds<Pixels>) -> Self {
        Self {
            strategy: Strategy::Side {
                trigger_bounds,
                placement: None,
                align: Align::Center,
                offset: px(0.),
            },
            corner_position: None,
            corner_around: None,
            aligned_item: None,
            on_position: None,
            margin: px(4.),
            approach: px(0.),
            occlude: false,
            children: Vec::new(),
        }
    }

    /// Places `anchor`'s corner of the popup at `position`.
    ///
    /// This is the corner-anchoring path used by triggers that were written
    /// against GPUI's `anchored` element. It clamps into the viewport without
    /// changing the requested anchor.
    pub fn corner(anchor: Anchor, position: Point<Pixels>) -> Self {
        Self {
            strategy: Strategy::Corner {
                anchor,
                position,
                around: None,
            },
            corner_position: None,
            corner_around: None,
            aligned_item: None,
            on_position: None,
            margin: px(4.),
            approach: px(0.),
            occlude: false,
            children: Vec::new(),
        }
    }

    /// Updates the requested corner position.
    ///
    /// This is useful when an animation moves an already-composed popup. It
    /// has no effect on a side-positioned popup.
    pub fn position(mut self, position: Point<Pixels>) -> Self {
        if let Strategy::Corner {
            position: current, ..
        } = &mut self.strategy
        {
            *current = position;
        }
        self
    }

    /// Lets a corner-anchored popup flip to the other side of `trigger`, the
    /// bounds its position was taken from, when it does not fit on its own
    /// side and the other has more room. Only meaningful for
    /// [`Positioner::corner`].
    pub fn flip_around(mut self, trigger: Bounds<Pixels>) -> Self {
        if let Strategy::Corner { around, .. } = &mut self.strategy {
            *around = Some(trigger);
        }
        self
    }

    /// Sets the preferred side. Only meaningful for [`Positioner::side`].
    pub fn placement(mut self, placement: Placement) -> Self {
        if let Strategy::Side {
            placement: slot, ..
        } = &mut self.strategy
        {
            *slot = Some(placement);
        }
        self
    }

    /// Sets the alignment along the chosen side. Only meaningful for
    /// [`Positioner::side`].
    pub fn align(mut self, align: Align) -> Self {
        if let Strategy::Side { align: slot, .. } = &mut self.strategy {
            *slot = align;
        }
        self
    }

    /// Sets the gap between the trigger and the popup. Only meaningful for
    /// [`Positioner::side`].
    pub fn offset(mut self, offset: Pixels) -> Self {
        if let Strategy::Side { offset: slot, .. } = &mut self.strategy {
            *slot = offset;
        }
        self
    }

    /// Lines an item of the popup up with the trigger, as Base UI's Select
    /// positioner does with `alignItemWithTrigger`.
    ///
    /// The popup is laid over the trigger so that the item `item` describes
    /// is centred on it, which puts the item's text where the trigger's value
    /// is. The popup's list is scrolled as far as that takes, reported as
    /// [`ResolvedPosition::item_scroll`] for the caller to apply, and the
    /// popup stays the margin inside the viewport. When there is not room to
    /// do that with the whole item in view, or `item` returns `None`, the
    /// popup is placed on its side of the trigger as it would be without this.
    ///
    /// `item` is asked while the popup is laid out, so it can read geometry
    /// that only exists once the popup's content has been laid out. Only
    /// meaningful for [`Positioner::side`], whose alignment still sets where
    /// the popup sits across the trigger.
    pub fn align_item_with_trigger(
        mut self,
        item: impl Fn(&App) -> Option<AlignedItem> + 'static,
    ) -> Self {
        self.aligned_item = Some(Box::new(item));
        self
    }

    // Read after trigger prepaint so an open popup follows a moving trigger.
    /// Reads the trigger [`Positioner::flip_around`] names from `around` as it
    /// lays out, as [`Positioner::tracked_corner_position`] does the position.
    pub(crate) fn tracked_flip_around(mut self, around: Rc<Cell<Bounds<Pixels>>>) -> Self {
        self.corner_around = Some(around);
        self
    }

    pub(crate) fn tracked_corner_position(mut self, position: Rc<Cell<Point<Pixels>>>) -> Self {
        self.corner_position = Some(position);
        self
    }

    /// Observe resolved geometry before children prepaint.
    pub fn on_position(mut self, callback: impl Fn(ResolvedPosition) + 'static) -> Self {
        self.on_position = Some(Box::new(callback));
        self
    }

    /// Blocks the mouse over the positioned popup.
    ///
    /// Off by default, because a tooltip that swallowed the pointer would
    /// un-hover the very trigger keeping it open. An interactive surface — a
    /// popover, a menu, a dropdown — wants it on: what the surface covers is
    /// the surface's, not the panel underneath.
    pub fn occlude(mut self) -> Self {
        self.occlude = true;
        self
    }

    /// Sets the minimum distance kept between the popup and the viewport edge.
    pub fn margin(mut self, margin: Pixels) -> Self {
        self.margin = margin;
        self
    }

    /// Draws the popup `distance` short of its place, back toward the trigger
    /// along the side it resolved to: up for a popup below its trigger, down
    /// for one that flipped above it, and likewise across. An enter motion
    /// animates this to zero, so the surface always comes out of the trigger's
    /// edge, whichever side it lands on.
    ///
    /// Only the drawing moves: [`Positioner::on_position`] still reports the
    /// place the popup settles at. A corner-positioned popup comes out of the
    /// corner it hangs from, the flipped one when it flipped.
    ///
    /// The approach is spatial motion, so reduced motion draws the popup in
    /// its place: a popup that keeps its fade there only fades.
    pub fn approach(mut self, distance: Pixels) -> Self {
        self.approach = distance;
        self
    }
}

/// How far back toward its trigger a popup on `placement`, or placed by its
/// corner `anchor`, is drawn for an [`Positioner::approach`] of `distance`.
fn approach_shift(
    placement: Option<Placement>,
    anchor: Option<Anchor>,
    distance: Pixels,
) -> Point<Pixels> {
    match (placement, anchor) {
        (Some(Placement::Bottom), _) => point(px(0.), -distance),
        (Some(Placement::Top), _) => point(px(0.), distance),
        (Some(Placement::Right), _) => point(-distance, px(0.)),
        (Some(Placement::Left), _) => point(distance, px(0.)),
        // A popup hung by its top corner opens below the point, by its bottom
        // corner above it.
        (None, Some(Anchor::TopLeft | Anchor::TopCenter | Anchor::TopRight)) => {
            point(px(0.), -distance)
        }
        (None, Some(Anchor::BottomLeft | Anchor::BottomCenter | Anchor::BottomRight)) => {
            point(px(0.), distance)
        }
        (None, Some(Anchor::LeftCenter)) => point(-distance, px(0.)),
        (None, Some(Anchor::RightCenter)) => point(distance, px(0.)),
        (None, None) => Point::default(),
    }
}

/// The part of the window's viewport that is frame rather than content, per
/// side.
///
/// A window drawn with client-side decorations pads its content by the client
/// inset to make room for a shadow, except along an edge that is tiled against
/// the screen, where the frame draws no shadow and the content runs to the
/// viewport edge. `Window::client_inset` is the one value on every side (the
/// platform needs it stable across tiling changes to size the window), so the
/// tiling is what says where it actually applies. A server-decorated window
/// has no frame of its own.
fn frame_insets(decorations: Decorations, client_inset: Pixels) -> Edges<Pixels> {
    match decorations {
        Decorations::Server => Edges::default(),
        Decorations::Client { tiling } => Edges {
            top: if tiling.top { px(0.) } else { client_inset },
            right: if tiling.right { px(0.) } else { client_inset },
            bottom: if tiling.bottom { px(0.) } else { client_inset },
            left: if tiling.left { px(0.) } else { client_inset },
        },
    }
}

/// Resolves the bounds of a popup of `popup_size`.
///
/// Side placement picks the preferred side when the popup fits, otherwise the
/// opposite side, otherwise whichever side has more room. The result is always
/// clamped into the viewport, keeping `margin` from each edge.
fn resolve(
    strategy: Strategy,
    popup_size: Size<Pixels>,
    viewport_size: Size<Pixels>,
    margin: Edges<Pixels>,
) -> ResolvedPosition {
    match strategy {
        Strategy::Corner {
            anchor,
            position,
            around,
        } => {
            let (anchor, position) = around.map_or((anchor, position), |trigger| {
                flip_corner(anchor, position, trigger, popup_size, viewport_size, margin)
            });
            ResolvedPosition {
                bounds: clamp(
                    Bounds::from_anchor_and_size(anchor, position, popup_size),
                    viewport_size,
                    margin,
                ),
                placement: None,
                anchor: Some(anchor),
                item_scroll: None,
            }
        }
        Strategy::Side {
            trigger_bounds,
            placement,
            align,
            offset,
        } => {
            let placement =
                resolve_placement(trigger_bounds, popup_size, viewport_size, margin, placement);
            let origin = side_origin(trigger_bounds, popup_size, placement, align, offset);
            ResolvedPosition {
                bounds: clamp(Bounds::new(origin, popup_size), viewport_size, margin),
                placement: Some(placement),
                anchor: None,
                item_scroll: None,
            }
        }
    }
}

/// Resolves an item-aligned popup of `popup_size`: the item centred on the
/// trigger, the list scrolled within its range to get it there, the whole
/// item in view and the popup inside the viewport by `margin`. `None` when no
/// scroll satisfies all of that, so the caller places the popup on a side.
///
/// Of the scrolls that work, the one that centres the item in the popup is
/// taken, so a long list opens with as much of it around the item as fits.
fn resolve_aligned_item(
    trigger_bounds: Bounds<Pixels>,
    align: Align,
    item: AlignedItem,
    popup_size: Size<Pixels>,
    viewport_size: Size<Pixels>,
    margin: Edges<Pixels>,
) -> Option<ResolvedPosition> {
    let top_limit = margin.top;
    let bottom_limit = (viewport_size.height - margin.bottom).max(margin.top);
    let height = popup_size.height;
    if height > bottom_limit - top_limit || item.height > height {
        return None;
    }

    // Where the item's top edge has to be for it to be centred on the
    // trigger. With the list scrolled by `scroll`, the popup's top edge is
    // then `item_y - item.top + scroll`.
    let item_y = trigger_bounds.center().y - item.height.half();
    let max_scroll = (item.content_height - height).max(px(0.));
    let lowest = px(0.)
        .max(top_limit - item_y + item.top)
        .max(item.top + item.height - height);
    let highest = max_scroll
        .min(bottom_limit - height - item_y + item.top)
        .min(item.top);
    if lowest > highest {
        return None;
    }

    let centred = item.top + item.height.half() - height.half();
    let scroll = centred
        .clamp(lowest, highest)
        .round()
        .clamp(lowest, highest);
    let x = match align {
        Align::Start => trigger_bounds.left(),
        Align::Center => trigger_bounds.center().x - popup_size.width.half(),
        Align::End => trigger_bounds.right() - popup_size.width,
    };
    let origin = point(x, item_y - item.top + scroll);
    Some(ResolvedPosition {
        bounds: clamp(Bounds::new(origin, popup_size), viewport_size, margin),
        placement: None,
        anchor: None,
        item_scroll: Some(scroll),
    })
}

/// The anchor and position a corner-anchored popup takes: its own when it fits
/// on its side of `trigger` (below it for a top anchor, above for a bottom one,
/// right of it for a left one, left for a right one), else the mirror across
/// the trigger, keeping the gap, when the other side has more room.
fn flip_corner(
    anchor: Anchor,
    position: Point<Pixels>,
    trigger: Bounds<Pixels>,
    popup_size: Size<Pixels>,
    viewport_size: Size<Pixels>,
    margin: Edges<Pixels>,
) -> (Anchor, Point<Pixels>) {
    let right_limit = (viewport_size.width - margin.right).max(margin.left);
    let bottom_limit = (viewport_size.height - margin.bottom).max(margin.top);
    // (room on its side, room on the other, the mirrored anchor and position)
    let (room, other_room, mirrored) = match anchor {
        Anchor::TopLeft | Anchor::TopCenter | Anchor::TopRight => {
            let y = trigger.top() - (position.y - trigger.bottom());
            let mirror = match anchor {
                Anchor::TopLeft => Anchor::BottomLeft,
                Anchor::TopCenter => Anchor::BottomCenter,
                _ => Anchor::BottomRight,
            };
            (
                bottom_limit - position.y,
                y - margin.top,
                (mirror, point(position.x, y)),
            )
        }
        Anchor::BottomLeft | Anchor::BottomCenter | Anchor::BottomRight => {
            let y = trigger.bottom() + (trigger.top() - position.y);
            let mirror = match anchor {
                Anchor::BottomLeft => Anchor::TopLeft,
                Anchor::BottomCenter => Anchor::TopCenter,
                _ => Anchor::TopRight,
            };
            (
                position.y - margin.top,
                bottom_limit - y,
                (mirror, point(position.x, y)),
            )
        }
        Anchor::LeftCenter => {
            let x = trigger.left() - (position.x - trigger.right());
            let mirrored = (Anchor::RightCenter, point(x, position.y));
            (right_limit - position.x, x - margin.left, mirrored)
        }
        Anchor::RightCenter => {
            let x = trigger.right() + (trigger.left() - position.x);
            let mirrored = (Anchor::LeftCenter, point(x, position.y));
            (position.x - margin.left, right_limit - x, mirrored)
        }
    };
    let extent = match anchor {
        Anchor::LeftCenter | Anchor::RightCenter => popup_size.width,
        _ => popup_size.height,
    };
    if extent > room && other_room > room {
        mirrored
    } else {
        (anchor, position)
    }
}

fn resolve_placement(
    trigger_bounds: Bounds<Pixels>,
    popup_size: Size<Pixels>,
    viewport_size: Size<Pixels>,
    margin: Edges<Pixels>,
    preferred: Option<Placement>,
) -> Placement {
    let right_limit = (viewport_size.width - margin.right).max(margin.left);
    let bottom_limit = (viewport_size.height - margin.bottom).max(margin.top);
    let available_left = (trigger_bounds.left() - margin.left).max(px(0.));
    let available_right = (right_limit - trigger_bounds.right()).max(px(0.));
    let available_above = (trigger_bounds.top() - margin.top).max(px(0.));
    let available_below = (bottom_limit - trigger_bounds.bottom()).max(px(0.));

    match preferred {
        Some(Placement::Right) if popup_size.width <= available_right => Placement::Right,
        Some(Placement::Right) if popup_size.width <= available_left => Placement::Left,
        Some(Placement::Right) if available_right >= available_left => Placement::Right,
        Some(Placement::Right) => Placement::Left,
        Some(Placement::Left) if popup_size.width <= available_left => Placement::Left,
        Some(Placement::Left) if popup_size.width <= available_right => Placement::Right,
        Some(Placement::Left) if available_left >= available_right => Placement::Left,
        Some(Placement::Left) => Placement::Right,
        Some(Placement::Bottom) if popup_size.height <= available_below => Placement::Bottom,
        Some(Placement::Bottom) if popup_size.height <= available_above => Placement::Top,
        Some(Placement::Bottom) if available_below >= available_above => Placement::Bottom,
        Some(Placement::Bottom) => Placement::Top,
        Some(Placement::Top) | None if popup_size.height <= available_above => Placement::Top,
        Some(Placement::Top) | None if popup_size.height <= available_below => Placement::Bottom,
        Some(Placement::Top) | None if available_below >= available_above => Placement::Bottom,
        Some(Placement::Top) | None => Placement::Top,
    }
}

fn side_origin(
    trigger_bounds: Bounds<Pixels>,
    popup_size: Size<Pixels>,
    placement: Placement,
    align: Align,
    offset: Pixels,
) -> Point<Pixels> {
    let aligned_x = match align {
        Align::Start => trigger_bounds.left(),
        Align::Center => trigger_bounds.center().x - popup_size.width.half(),
        Align::End => trigger_bounds.right() - popup_size.width,
    };
    let aligned_y = match align {
        Align::Start => trigger_bounds.top(),
        Align::Center => trigger_bounds.center().y - popup_size.height.half(),
        Align::End => trigger_bounds.bottom() - popup_size.height,
    };

    match placement {
        Placement::Top => point(aligned_x, trigger_bounds.top() - popup_size.height - offset),
        Placement::Bottom => point(aligned_x, trigger_bounds.bottom() + offset),
        Placement::Left => point(trigger_bounds.left() - popup_size.width - offset, aligned_y),
        Placement::Right => point(trigger_bounds.right() + offset, aligned_y),
    }
}

fn clamp(
    mut bounds: Bounds<Pixels>,
    viewport_size: Size<Pixels>,
    margin: Edges<Pixels>,
) -> Bounds<Pixels> {
    let right_limit = (viewport_size.width - margin.right).max(margin.left);
    let bottom_limit = (viewport_size.height - margin.bottom).max(margin.top);

    if bounds.right() > right_limit {
        bounds.origin.x -= bounds.right() - right_limit;
    }
    if bounds.left() < margin.left {
        bounds.origin.x = margin.left;
    }
    if bounds.bottom() > bottom_limit {
        bounds.origin.y -= bounds.bottom() - bottom_limit;
    }
    if bounds.top() < margin.top {
        bounds.origin.y = margin.top;
    }

    bounds
}

impl ParentElement for Positioner {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Element for Positioner {
    type RequestLayoutState = PositionerState;
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let child_layout_ids = self
            .children
            .iter_mut()
            .map(|child| child.request_layout(window, cx))
            .collect::<Vec<_>>();
        let layout_id = window.request_layout(
            Style {
                position: Position::Absolute,
                display: Display::Flex,
                ..Style::default()
            },
            child_layout_ids.iter().copied(),
            cx,
        );

        (layout_id, PositionerState { child_layout_ids })
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if request_layout.child_layout_ids.is_empty() {
            return;
        }

        let mut child_min = point(Pixels::MAX, Pixels::MAX);
        let mut child_max = Point::default();
        for child_layout_id in &request_layout.child_layout_ids {
            let child_bounds = window.layout_bounds(*child_layout_id);
            child_min = child_min.min(&child_bounds.origin);
            child_max = child_max.max(&child_bounds.bottom_right());
        }

        let popup_size = (child_max - child_min).into();
        let frame = frame_insets(
            window.window_decorations(),
            window.client_inset().unwrap_or(px(0.)),
        );
        let mut strategy = self.strategy;
        if let (Strategy::Corner { position, .. }, Some(tracked)) =
            (&mut strategy, &self.corner_position)
        {
            *position = tracked.get();
        }
        if let (Strategy::Corner { around, .. }, Some(tracked)) =
            (&mut strategy, &self.corner_around)
        {
            *around = Some(tracked.get());
        }
        let viewport_size = window.viewport_size();
        let margin = frame.map(|inset| *inset + self.margin);
        let aligned = match (&self.aligned_item, strategy) {
            (
                Some(item),
                Strategy::Side {
                    trigger_bounds,
                    align,
                    ..
                },
            ) => item(&*cx).and_then(|item| {
                resolve_aligned_item(
                    trigger_bounds,
                    align,
                    item,
                    popup_size,
                    viewport_size,
                    margin,
                )
            }),
            _ => None,
        };
        let position =
            aligned.unwrap_or_else(|| resolve(strategy, popup_size, viewport_size, margin));
        if let Some(callback) = &self.on_position {
            callback(position);
        }
        // Ahead of the children so it blocks what is behind the popup without
        // blocking the popup's own content.
        if self.occlude {
            window.insert_hitbox(position.bounds, HitboxBehavior::BlockMouse);
        }

        let approach = if cx.reduce_motion() {
            px(0.)
        } else {
            self.approach
        };
        let shift = approach_shift(position.placement, position.anchor, approach);
        let offset = position.bounds.origin + shift - bounds.origin;
        let offset = point(offset.x.round(), offset.y.round());

        window.with_element_offset(offset, |window| {
            for child in &mut self.children {
                child.prepaint(window, cx);
            }
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        for child in &mut self.children {
            child.paint(window, cx);
        }
    }
}

impl IntoElement for Positioner {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Tiling;

    const MARGIN: Pixels = px(4.);

    fn trigger(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), Size::new(px(w), px(h)))
    }

    fn viewport() -> Size<Pixels> {
        Size::new(px(500.), px(400.))
    }

    fn side(
        trigger_bounds: Bounds<Pixels>,
        placement: Option<Placement>,
        align: Align,
        popup: Size<Pixels>,
    ) -> ResolvedPosition {
        resolve(
            Strategy::Side {
                trigger_bounds,
                placement,
                align,
                offset: px(0.),
            },
            popup,
            viewport(),
            Edges::all(MARGIN),
        )
    }

    #[test]
    fn prefers_the_requested_side_when_it_fits() {
        let position = side(
            trigger(200., 200., 40., 20.),
            Some(Placement::Top),
            Align::Center,
            Size::new(px(80.), px(30.)),
        );

        assert_eq!(position.placement, Some(Placement::Top));
        assert_eq!(position.bounds.bottom(), px(200.));
    }

    #[test]
    fn an_approach_comes_from_the_trigger_on_every_side() {
        let d = px(8.);
        assert_eq!(
            approach_shift(Some(Placement::Bottom), None, d),
            point(px(0.), px(-8.))
        );
        assert_eq!(
            approach_shift(Some(Placement::Top), None, d),
            point(px(0.), px(8.))
        );
        assert_eq!(
            approach_shift(Some(Placement::Right), None, d),
            point(px(-8.), px(0.))
        );
        assert_eq!(
            approach_shift(Some(Placement::Left), None, d),
            point(px(8.), px(0.))
        );
        assert_eq!(
            approach_shift(None, Some(Anchor::TopLeft), d),
            point(px(0.), px(-8.)),
            "a popup hung below a corner comes down out of it"
        );
        assert_eq!(
            approach_shift(None, Some(Anchor::BottomRight), d),
            point(px(0.), px(8.))
        );
    }

    #[test]
    fn flips_to_the_opposite_side_when_the_preferred_side_does_not_fit() {
        let position = side(
            trigger(200., 10., 40., 20.),
            Some(Placement::Top),
            Align::Center,
            Size::new(px(80.), px(60.)),
        );

        assert_eq!(position.placement, Some(Placement::Bottom));
        assert_eq!(position.bounds.top(), px(30.));
    }

    #[test]
    fn clamps_into_the_viewport_while_keeping_the_flipped_side() {
        let position = side(
            trigger(480., 200., 40., 20.),
            Some(Placement::Bottom),
            Align::Center,
            Size::new(px(120.), px(30.)),
        );

        assert_eq!(position.placement, Some(Placement::Bottom));
        assert_eq!(position.bounds.right(), viewport().width - MARGIN);
    }

    #[test]
    fn alignment_selects_the_leading_center_or_trailing_edge() {
        let trigger_bounds = trigger(200., 200., 100., 20.);
        let popup = Size::new(px(40.), px(30.));

        let start = side(trigger_bounds, Some(Placement::Bottom), Align::Start, popup);
        let center = side(
            trigger_bounds,
            Some(Placement::Bottom),
            Align::Center,
            popup,
        );
        let end = side(trigger_bounds, Some(Placement::Bottom), Align::End, popup);

        assert_eq!(start.bounds.left(), px(200.));
        assert_eq!(center.bounds.left(), px(230.));
        assert_eq!(end.bounds.left(), px(260.));
    }

    #[test]
    fn side_offset_adds_a_gap_between_trigger_and_popup() {
        let position = resolve(
            Strategy::Side {
                trigger_bounds: trigger(200., 200., 40., 20.),
                placement: Some(Placement::Bottom),
                align: Align::Center,
                offset: px(8.),
            },
            Size::new(px(40.), px(30.)),
            viewport(),
            Edges::all(MARGIN),
        );

        assert_eq!(position.bounds.top(), px(228.));
    }

    #[test]
    fn corner_positioning_places_the_named_corner_and_never_reports_a_side() {
        let position = resolve(
            Strategy::Corner {
                anchor: Anchor::TopLeft,
                position: point(px(100.), px(100.)),
                around: None,
            },
            Size::new(px(40.), px(30.)),
            viewport(),
            Edges::all(MARGIN),
        );

        assert_eq!(position.placement, None);
        assert_eq!(position.bounds.origin, point(px(100.), px(100.)));
    }

    #[test]
    fn corner_positioning_clamps_but_does_not_flip() {
        let position = resolve(
            Strategy::Corner {
                anchor: Anchor::TopLeft,
                position: point(px(480.), px(390.)),
                around: None,
            },
            Size::new(px(40.), px(30.)),
            viewport(),
            Edges::all(MARGIN),
        );

        assert_eq!(position.placement, None);
        assert_eq!(position.bounds.right(), viewport().width - MARGIN);
        assert_eq!(position.bounds.bottom(), viewport().height - MARGIN);
    }

    #[test]
    fn corner_position_can_be_updated_without_rebuilding_the_positioner() {
        let positioner = Positioner::corner(Anchor::TopLeft, point(px(10.), px(20.)))
            .position(point(px(30.), px(40.)));

        assert!(matches!(
            positioner.strategy,
            Strategy::Corner {
                position,
                ..
            } if position == point(px(30.), px(40.))
        ));
    }

    /// A tiled edge draws no shadow, so a popup may run right up to the
    /// margin there; an untiled edge keeps the whole client inset as well.
    #[test]
    fn frame_insets_apply_the_client_inset_only_on_untiled_edges() {
        let inset = px(20.);

        assert_eq!(
            frame_insets(Decorations::Server, inset),
            Edges::default(),
            "a server-decorated window has no frame to keep clear of"
        );
        assert_eq!(
            frame_insets(
                Decorations::Client {
                    tiling: Tiling::tiled()
                },
                inset
            ),
            Edges::default(),
            "a window tiled on every side draws no shadow padding"
        );
        assert_eq!(
            frame_insets(
                Decorations::Client {
                    tiling: Tiling {
                        top: false,
                        left: true,
                        right: false,
                        bottom: true,
                    }
                },
                inset
            ),
            Edges {
                top: inset,
                right: inset,
                bottom: px(0.),
                left: px(0.),
            }
        );
    }

    /// The bug this guards: a menu opened from a trigger near the right edge
    /// of a tiled window was pushed inward by the client inset although no
    /// shadow was drawn there, so it no longer lined up with its trigger.
    #[test]
    fn clamping_keeps_only_the_margin_on_a_tiled_edge() {
        let popup = Size::new(px(120.), px(30.));
        // A menu aligned to a trigger that ends 10px short of the right edge.
        let corner = Strategy::Corner {
            anchor: Anchor::TopRight,
            position: point(viewport().width - px(10.), px(100.)),
            around: None,
        };
        let margin = Edges::all(MARGIN);
        let inset = px(20.);

        let tiled = resolve(corner, popup, viewport(), margin);
        assert_eq!(tiled.bounds.right(), viewport().width - px(10.));

        let untiled = resolve(
            corner,
            popup,
            viewport(),
            margin.map(|margin| *margin + inset),
        );
        assert_eq!(untiled.bounds.right(), viewport().width - MARGIN - inset);
    }

    // Migrated from the tooltip module when its private positioning logic was
    // merged into this one. They passed unchanged across that move, which is
    // what proves the merge preserved tooltip placement behavior.
    const WINDOW_MARGIN: Pixels = px(4.);

    fn tooltip_placement(
        trigger_bounds: Bounds<Pixels>,
        popup: Size<Pixels>,
        viewport: Size<Pixels>,
        margin: Pixels,
        placement: Option<Placement>,
    ) -> (Bounds<Pixels>, Placement) {
        let resolved = resolve(
            Strategy::Side {
                trigger_bounds,
                placement,
                align: Align::Center,
                offset: px(0.),
            },
            popup,
            viewport,
            Edges::all(margin),
        );
        (resolved.bounds, resolved.placement.unwrap())
    }

    fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), Size::new(px(width), px(height)))
    }

    fn size_px(width: f32, height: f32) -> Size<Pixels> {
        Size::new(px(width), px(height))
    }

    #[test]
    fn prefers_above_when_space_allows() {
        let trigger = bounds(100., 80., 80., 24.);
        let position = tooltip_placement(
            trigger,
            size_px(120., 30.),
            size_px(300., 200.),
            WINDOW_MARGIN,
            None,
        );
        assert_eq!(position.1, Placement::Top);
        assert_eq!(position.0.origin, point(px(80.), px(50.)));
    }
    #[test]
    fn flips_and_clamps_on_each_axis() {
        let top = tooltip_placement(
            bounds(24., 4., 120., 32.),
            size_px(240., 32.),
            size_px(520., 260.),
            WINDOW_MARGIN,
            None,
        );
        assert_eq!(top.1, Placement::Bottom);

        let right = tooltip_placement(
            bounds(260., 60., 32., 32.),
            size_px(120., 30.),
            size_px(300., 200.),
            WINDOW_MARGIN,
            Some(Placement::Right),
        );
        assert_eq!(right.1, Placement::Left);

        let left_edge = tooltip_placement(
            bounds(4., 80., 24., 24.),
            size_px(120., 30.),
            size_px(300., 200.),
            WINDOW_MARGIN,
            None,
        );
        assert_eq!(left_edge.0.left(), WINDOW_MARGIN);
    }
    #[test]
    fn places_tooltip_to_the_right() {
        let trigger = bounds(20., 60., 32., 32.);
        let position = tooltip_placement(
            trigger,
            size_px(120., 30.),
            size_px(300., 200.),
            WINDOW_MARGIN,
            Some(Placement::Right),
        );
        assert_eq!(position.1, Placement::Right);
        assert_eq!(position.0.left(), trigger.right());
        assert_eq!(position.0.center().y, trigger.center().y);
    }
    #[test]
    fn right_placement_clamps_vertical_edges() {
        let trigger = bounds(20., 2., 32., 20.);
        let position = tooltip_placement(
            trigger,
            size_px(120., 40.),
            size_px(300., 200.),
            WINDOW_MARGIN,
            Some(Placement::Right),
        );
        assert_eq!(position.1, Placement::Right);
        assert_eq!(position.0.top(), WINDOW_MARGIN);
        assert_eq!(position.0.left(), trigger.right());
    }
    #[test]
    fn uses_larger_side_when_neither_vertical_side_fits() {
        let position = tooltip_placement(
            bounds(120., 20., 40., 20.),
            size_px(160., 120.),
            size_px(300., 100.),
            WINDOW_MARGIN,
            None,
        );
        assert_eq!(position.1, Placement::Bottom);
        assert_eq!(position.0.top(), WINDOW_MARGIN);
        assert_eq!(position.0.left(), px(60.));
    }

    #[test]
    fn a_menu_near_the_bottom_opens_above_its_trigger() {
        // A 40 pt trigger whose foot is 30 pt above the viewport's, and a 100 pt
        // menu meant to hang 4 pt below it.
        let trigger = Bounds::new(point(px(100.), px(330.)), Size::new(px(80.), px(40.)));
        let position = resolve(
            Strategy::Corner {
                anchor: Anchor::TopLeft,
                position: point(px(100.), px(374.)),
                around: Some(trigger),
            },
            Size::new(px(120.), px(100.)),
            viewport(),
            Edges::all(MARGIN),
        );
        // Above it, the same 4 pt off, not slid up over it.
        assert_eq!(position.bounds.bottom(), px(326.));
        assert_eq!(position.bounds.left(), px(100.));
    }

    #[test]
    fn a_menu_that_fits_below_stays_below() {
        let trigger = Bounds::new(point(px(100.), px(100.)), Size::new(px(80.), px(40.)));
        let position = resolve(
            Strategy::Corner {
                anchor: Anchor::TopLeft,
                position: point(px(100.), px(144.)),
                around: Some(trigger),
            },
            Size::new(px(120.), px(100.)),
            viewport(),
            Edges::all(MARGIN),
        );
        assert_eq!(position.bounds.top(), px(144.));
    }

    #[test]
    fn a_menu_with_less_room_above_stays_below_and_is_clamped() {
        // Near the top, too tall for either side: it keeps the roomier side.
        let trigger = Bounds::new(point(px(100.), px(60.)), Size::new(px(80.), px(40.)));
        let position = resolve(
            Strategy::Corner {
                anchor: Anchor::TopLeft,
                position: point(px(100.), px(104.)),
                around: Some(trigger),
            },
            Size::new(px(120.), px(380.)),
            viewport(),
            Edges::all(MARGIN),
        );
        assert_eq!(position.bounds.bottom(), px(396.), "kept below, clamped");
    }

    #[test]
    fn a_side_menu_near_the_right_edge_opens_to_the_left() {
        let trigger = Bounds::new(point(px(400.), px(100.)), Size::new(px(60.), px(20.)));
        let position = resolve(
            Strategy::Corner {
                anchor: Anchor::LeftCenter,
                position: point(px(464.), px(110.)),
                around: Some(trigger),
            },
            Size::new(px(150.), px(80.)),
            viewport(),
            Edges::all(MARGIN),
        );
        assert_eq!(position.bounds.right(), px(396.));
    }

    /// Eight 32 pt rows inside 4 pt of padding, which all fit.
    fn short_list_row(row: f32) -> AlignedItem {
        AlignedItem::new(px(4. + row * 32.), px(32.)).with_content_height(px(264.))
    }

    fn aligned(
        trigger_bounds: Bounds<Pixels>,
        item: AlignedItem,
        popup: Size<Pixels>,
    ) -> Option<ResolvedPosition> {
        resolve_aligned_item(
            trigger_bounds,
            Align::Start,
            item,
            popup,
            viewport(),
            Edges::all(MARGIN),
        )
    }

    #[test]
    fn an_aligned_item_is_centred_on_the_trigger() {
        let trigger_bounds = trigger(100., 200., 120., 36.);
        let item = short_list_row(3.);
        let position = aligned(trigger_bounds, item, Size::new(px(120.), px(264.)))
            .expect("there is room to align the item");

        assert_eq!(position.item_scroll, Some(px(0.)), "a list that fits");
        assert_eq!(position.bounds.left(), trigger_bounds.left());
        let item_top = position.bounds.top() + item.top();
        assert_eq!(
            item_top + item.height() * 0.5,
            trigger_bounds.center().y,
            "the item sits exactly over the trigger"
        );
        assert_eq!(position.placement, None);
    }

    #[test]
    fn an_aligned_item_scrolls_the_list_to_keep_the_popup_in_the_viewport() {
        // Twenty rows in a popup cut to 300 pt, below a trigger near the top:
        // the popup cannot rise far enough to put row 10 over the trigger
        // unscrolled, so the list scrolls instead.
        let trigger_bounds = trigger(100., 40., 120., 36.);
        let item = AlignedItem::new(px(4. + 10. * 32.), px(32.)).with_content_height(px(648.));
        let position = aligned(trigger_bounds, item, Size::new(px(120.), px(300.)))
            .expect("the list scrolls far enough");

        let scroll = position.item_scroll.unwrap();
        assert!(scroll > px(0.));
        assert!(position.bounds.top() >= MARGIN, "inside the viewport");
        assert_eq!(
            position.bounds.top() + item.top() - scroll + item.height() * 0.5,
            trigger_bounds.center().y
        );
    }

    #[test]
    fn an_item_that_cannot_reach_the_trigger_is_not_aligned() {
        // A list that does not scroll, with row 5 under a trigger at the top
        // of the window: the popup would have to start above the viewport.
        let trigger_bounds = trigger(100., 10., 120., 36.);
        assert_eq!(
            aligned(
                trigger_bounds,
                short_list_row(5.),
                Size::new(px(120.), px(264.))
            ),
            None
        );
    }

    #[test]
    fn a_popup_taller_than_the_viewport_is_not_aligned() {
        let trigger_bounds = trigger(100., 200., 120., 36.);
        assert_eq!(
            aligned(
                trigger_bounds,
                short_list_row(0.),
                Size::new(px(120.), px(600.))
            ),
            None
        );
    }
}
