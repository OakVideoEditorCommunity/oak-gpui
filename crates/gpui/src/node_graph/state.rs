//! Viewport and selection state for the node-graph editor.
//!
//! [`GraphViewState`] owns everything about *how* the graph is looked at —
//! pan offset, zoom, selection, marquee — and nothing about the graph itself.
//! The coordinate transforms here are pure and implemented; they are the
//! single source of truth for the graph-space ↔ screen-space mapping used by
//! node rendering, wire anchoring and hit testing alike.

use std::collections::BTreeSet;

use crate::{Bounds, Pixels, Point, Size, point};

use crate::node_graph::NodeId;

/// Minimum zoom factor accepted by [`GraphViewState::set_zoom`] and
/// [`GraphViewState::zoom_at`]: the graph is shown at 10% scale.
pub const MIN_ZOOM: f32 = 0.1;

/// Maximum zoom factor accepted by [`GraphViewState::set_zoom`] and
/// [`GraphViewState::zoom_at`]: the graph is shown at 400% scale.
pub const MAX_ZOOM: f32 = 4.0;

/// Pan/zoom viewport and selection state of a
/// [`NodeGraphView`](crate::node_graph::NodeGraphView).
///
/// # Coordinate spaces
///
/// - *Graph space* is the unbounded document coordinate system that
///   [`NodeData::position`](crate::node_graph::NodeData::position) returns.
/// - *Screen space* is the element-local pixel coordinate system used for
///   painting and hit testing, with the origin at the top-left corner of the
///   graph view.
///
/// The mapping is an affine transform with no rotation:
///
/// ```text
/// screen = graph * zoom + offset
/// graph  = (screen - offset) / zoom
/// ```
#[derive(Clone, Debug)]
pub struct GraphViewState {
    /// Pan offset in screen space: the screen-space position of the graph
    /// origin. Positive values move the graph content down-right.
    offset: Point<Pixels>,
    /// Zoom factor, always within [`MIN_ZOOM`]..=[`MAX_ZOOM`]. `1.0` is 100%.
    zoom: f32,
    /// The currently selected nodes. Kept sorted (B-Tree) so that
    /// `SelectionChanged` events are deterministic and cheap to diff.
    selection: BTreeSet<NodeId>,
    /// An in-progress marquee (rubber-band) selection rectangle, in screen
    /// space, if the user is currently dragging one.
    marquee: Option<SelectionRect>,
}

impl Default for GraphViewState {
    fn default() -> Self {
        Self {
            offset: point(Pixels::ZERO, Pixels::ZERO),
            zoom: 1.0,
            selection: BTreeSet::new(),
            marquee: None,
        }
    }
}

impl GraphViewState {
    /// Creates a fresh view state: no pan, 100% zoom, empty selection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the current pan offset (the screen-space position of the
    /// graph origin).
    pub fn offset(&self) -> Point<Pixels> {
        self.offset
    }

    /// Sets the pan offset directly. No clamping is applied — the graph is
    /// unbounded.
    pub fn set_offset(&mut self, offset: Point<Pixels>) {
        self.offset = offset;
    }

    /// Pans the view by a screen-space delta (typically a drag delta).
    pub fn pan_by(&mut self, delta: Point<Pixels>) {
        self.offset = self.offset + delta;
    }

    /// Returns the current zoom factor, guaranteed within
    /// [`MIN_ZOOM`]..=[`MAX_ZOOM`].
    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// Sets the zoom factor, clamped to [`MIN_ZOOM`]..=[`MAX_ZOOM`].
    ///
    /// Unlike [`zoom_at`](Self::zoom_at) this does not preserve any anchor
    /// point; the graph origin stays put and content scales around it.
    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    }

    /// Zooms by `factor` (e.g. `1.1` per scroll step) while keeping the
    /// graph point under `anchor` (a screen-space position, usually the
    /// cursor) stationary on screen.
    ///
    /// # Math contract
    ///
    /// Let `z` be the old zoom and `z' = clamp(z * factor, MIN_ZOOM,
    /// MAX_ZOOM)` the new one. The offset is adjusted so that
    /// `graph_to_screen(g)` is identical before and after for the graph point
    /// `g = screen_to_graph(anchor)`:
    ///
    /// ```text
    /// offset' = anchor - (anchor - offset) * (z' / z)
    /// ```
    ///
    /// When the zoom is clamped (already at the min/max), `z' == z` and the
    /// offset is left untouched — the call is then a no-op.
    pub fn zoom_at(&mut self, anchor: Point<Pixels>, factor: f32) {
        let new_zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        if new_zoom == self.zoom {
            return;
        }
        let scale = new_zoom / self.zoom;
        self.offset = point(
            anchor.x - (anchor.x - self.offset.x) * scale,
            anchor.y - (anchor.y - self.offset.y) * scale,
        );
        self.zoom = new_zoom;
    }

    /// Fits the graph-space rectangle `rect` (typically the union of every
    /// node's bounds) into the `viewport` screen-space size: zooms so the
    /// rect occupies at most 95% of the viewport (clamped to
    /// [`MIN_ZOOM`]..=[`MAX_ZOOM`]) and pans so the rect is centered.
    ///
    /// No-op when either size is non-positive. Used by hosts for a "fit
    /// window" command and as the initial viewport after the first layout.
    pub fn fit_to_rect(&mut self, rect: Bounds<Pixels>, viewport: Size<Pixels>) {
        const PADDING: f32 = 40.0;
        let (rw, rh) = (rect.size.width.0, rect.size.height.0);
        let (vw, vh) = (viewport.width.0, viewport.height.0);
        if rw <= 0.0 || rh <= 0.0 || vw <= 0.0 || vh <= 0.0 {
            return;
        }
        // Fit the larger axis; the padding keeps a breathing margin.
        let zoom = (vw / (rw + PADDING * 2.0))
            .min(vh / (rh + PADDING * 2.0))
            .clamp(MIN_ZOOM, MAX_ZOOM);
        // Center the rect: offset = (viewport - rect_size * zoom) / 2
        //                       - rect_origin * zoom.
        self.zoom = zoom;
        self.offset = point(
            Pixels((vw - rw * zoom) * 0.5 - rect.origin.x.0 * zoom),
            Pixels((vh - rh * zoom) * 0.5 - rect.origin.y.0 * zoom),
        );
    }

    /// Maps a graph-space (document) point to screen space:
    /// `screen = graph * zoom + offset`.
    pub fn graph_to_screen(&self, graph: Point<Pixels>) -> Point<Pixels> {
        point(
            graph.x * self.zoom + self.offset.x,
            graph.y * self.zoom + self.offset.y,
        )
    }

    /// Maps a screen-space point to graph space:
    /// `graph = (screen - offset) / zoom`. This is the exact inverse of
    /// [`graph_to_screen`](Self::graph_to_screen).
    pub fn screen_to_graph(&self, screen: Point<Pixels>) -> Point<Pixels> {
        point(
            (screen.x - self.offset.x) / self.zoom,
            (screen.y - self.offset.y) / self.zoom,
        )
    }

    /// Returns the set of currently selected nodes.
    pub fn selection(&self) -> &BTreeSet<NodeId> {
        &self.selection
    }

    /// Returns whether the given node is currently selected.
    pub fn is_selected(&self, node: NodeId) -> bool {
        self.selection.contains(&node)
    }

    /// Replaces the selection with exactly the given nodes.
    ///
    /// The view compares before/after and emits
    /// [`NodeGraphEvent::SelectionChanged`](crate::node_graph::NodeGraphEvent::SelectionChanged)
    /// when the set actually changed; calling this directly does not emit
    /// events on its own.
    pub fn set_selection(&mut self, nodes: impl IntoIterator<Item = NodeId>) {
        self.selection = nodes.into_iter().collect();
    }

    /// Adds `node` to the selection (shift-click semantics).
    pub fn select(&mut self, node: NodeId) {
        self.selection.insert(node);
    }

    /// Removes `node` from the selection; returns whether it was selected.
    pub fn deselect(&mut self, node: NodeId) -> bool {
        self.selection.remove(&node)
    }

    /// Toggles `node` in the selection (shift-click toggle semantics).
    pub fn toggle_selection(&mut self, node: NodeId) {
        if !self.deselect(node) {
            self.select(node);
        }
    }

    /// Clears the selection.
    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    /// Returns the in-progress marquee selection rectangle, if any.
    pub fn marquee(&self) -> Option<&SelectionRect> {
        self.marquee.as_ref()
    }

    /// Begins a marquee selection anchored at the given screen-space point.
    pub fn begin_marquee(&mut self, anchor: Point<Pixels>) {
        self.marquee = Some(SelectionRect {
            anchor,
            current: anchor,
        });
    }

    /// Updates the current corner of the in-progress marquee. Does nothing if
    /// no marquee is in progress.
    pub fn update_marquee(&mut self, current: Point<Pixels>) {
        if let Some(marquee) = &mut self.marquee {
            marquee.current = current;
        }
    }

    /// Ends the marquee and returns it, or `None` if none was in progress.
    ///
    /// The caller (the view) converts the rect to graph space and selects all
    /// nodes intersecting it.
    pub fn end_marquee(&mut self) -> Option<SelectionRect> {
        self.marquee.take()
    }
}

/// A marquee (rubber-band) selection rectangle in screen space.
///
/// The rectangle is defined by the point where the drag started and the
/// current cursor position; use [`normalized`](Self::normalized) to obtain a
/// well-ordered rect regardless of drag direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionRect {
    /// The screen-space point where the marquee drag started.
    pub anchor: Point<Pixels>,
    /// The current screen-space corner (usually the cursor position).
    pub current: Point<Pixels>,
}

impl SelectionRect {
    /// Returns the axis-aligned rectangle with `min` as the top-left and
    /// `max` as the bottom-right corner, independent of drag direction.
    pub fn normalized(&self) -> (Point<Pixels>, Point<Pixels>) {
        let min = point(self.anchor.x.min(self.current.x), self.anchor.y.min(self.current.y));
        let max = point(self.anchor.x.max(self.current.x), self.anchor.y.max(self.current.y));
        (min, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{px, size};

    /// Fitting a graph rect into a viewport centers it and picks a zoom that
    /// fits the larger axis; the mapping must stay consistent afterwards.
    #[test]
    fn fit_centers_and_fits_the_rect() {
        let mut state = GraphViewState::new();
        let rect = Bounds::new(point(px(40.0), px(60.0)), size(px(1040.0), px(230.0)));
        state.fit_to_rect(rect, size(px(640.0), px(500.0)));

        // The rect's center must map to the viewport's center.
        let graph_center = rect.center();
        let screen_center = state.graph_to_screen(graph_center);
        assert!((screen_center.x.0 - 320.0).abs() < 0.5, "x center: {}", screen_center.x.0);
        assert!((screen_center.y.0 - 250.0).abs() < 0.5, "y center: {}", screen_center.y.0);

        // The fitted rect must fit within the viewport (with the 40px padding).
        let top_left = state.graph_to_screen(rect.origin);
        let bottom_right = state.graph_to_screen(rect.bottom_right());
        assert!(top_left.x.0 >= 0.0 && bottom_right.x.0 <= 640.0);
        assert!(top_left.y.0 >= 0.0 && bottom_right.y.0 <= 500.0);
    }

    /// The width and height both shrink when the rect is tall and wide
    /// (whichever axis is more constraining drives the zoom).
    #[test]
    fn fit_respects_both_axes() {
        let mut state = GraphViewState::new();
        // A wide rect in a narrow viewport: width drives the zoom.
        let rect = Bounds::new(point(px(0.0), px(0.0)), size(px(2000.0), px(100.0)));
        state.fit_to_rect(rect, size(px(400.0), px(400.0)));
        let fitted = state.graph_to_screen(rect.bottom_right());
        assert!(fitted.x.0 <= 400.0 && fitted.y.0 <= 400.0);
        assert!(state.zoom() < 1.0);
    }

    /// A rect smaller than the viewport zooms in (clamped to [`MAX_ZOOM`]).
    #[test]
    fn fit_zooms_in_for_small_graphs() {
        let mut state = GraphViewState::new();
        let rect = Bounds::new(point(px(0.0), px(0.0)), size(px(100.0), px(60.0)));
        state.fit_to_rect(rect, size(px(1000.0), px(800.0)));
        assert_eq!(state.zoom(), MAX_ZOOM);
    }

    /// Non-positive viewport or rect sizes are ignored.
    #[test]
    fn fit_ignores_non_positive_sizes() {
        let mut state = GraphViewState::new();
        let before = state.clone();
        let rect = Bounds::new(point(px(0.0), px(0.0)), size(px(100.0), px(60.0)));
        state.fit_to_rect(rect, size(px(0.0), px(800.0)));
        assert_eq!(state.zoom(), before.zoom());
        assert_eq!(state.offset(), before.offset());
    }
}
