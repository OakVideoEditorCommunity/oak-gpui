//! The draggable divider between the children of a
//! [`DockNode::Split`](crate::dock::DockNode::Split).
//!
//! Internal component — not part of the public API. One `SplitHandle` is
//! rendered between each pair of split children; dragging it adjusts the
//! split's `ratio` through
//! [`DockLayout::resize_split`](crate::dock::DockLayout::resize_split).

use crate::colors::DefaultColors;
use crate::{
	App, AppContext, Axis, ClickEvent, Context, ElementId, EventEmitter, InteractiveElement,
	IntoElement, Pixels, Point, Render, StatefulInteractiveElement, Styled, Window, div, px,
};

use super::{NodePath, path_key};

/// Events emitted by a [`SplitHandle`] toward its owning [`DockArea`].
#[derive(Clone, Debug)]
pub(crate) enum SplitHandleEvent {
	/// The user dragged the handle; the boundary at `index` of the split at
	/// `path` should be resized to `ratio` (child `index`'s share of the
	/// pair, already clamped to the allowed range).
	ResizeRequested {
		/// Path of the split node to resize.
		path: NodePath,
		/// The boundary being moved: between children `index` and `index + 1`.
		index: usize,
		/// Desired new ratio, already clamped to the allowed range.
		ratio: f32,
	},
	/// The user double-clicked the handle; the boundary at `index` of the
	/// split at `path` should be reset to [`SplitHandle::RESET_RATIO`].
	ResetRequested {
		/// Path of the split node to resize.
		path: NodePath,
		/// The boundary being moved: between children `index` and `index + 1`.
		index: usize,
	},
}

/// The payload of a handle drag; carried by the drag-and-drop system so the
/// split container can identify which handle is being dragged.
#[derive(Clone, Debug)]
pub(crate) struct SplitHandleDrag {
	/// Path of the split being resized.
	pub(crate) path: NodePath,
	/// The boundary being dragged: between children `index` and `index + 1`.
	pub(crate) index: usize,
}

/// A resize handle between two children of a split node.
///
/// # Behavior
///
/// - Renders as a thin divider along the split's cross axis with the matching
///   resize cursor (`col-resize` for [`Axis::Horizontal`] splits,
///   `row-resize` for [`Axis::Vertical`]).
/// - Dragging converts the pointer delta into a ratio delta (pixels of the
///   parent extent → fraction) and emits
///   [`SplitHandleEvent::ResizeRequested`]; the owning
///   [`DockArea`](crate::dock::DockArea) applies it via
///   [`DockLayout::resize_split_child`](crate::dock::DockLayout::resize_split_child),
///   clamping so neither side shrinks below
///   [`SplitHandle::MIN_CHILD_EXTENT`].
/// - Double-clicking emits [`SplitHandleEvent::ResetRequested`] to reset the
///   boundary to an even 50/50.
///
/// A split with N children renders N-1 handles (one per boundary), so each
/// pair of panels can be resized independently while the others keep their
/// ratios. The handle carries the [`NodePath`] of its split and the boundary
/// `index` so it can address the correct node after unrelated edits elsewhere
/// in the tree; paths are re-derived on every render, never stored across
/// frames.
pub(crate) struct SplitHandle {
	/// Axis along which the parent split lays out its children; the handle
	/// itself extends along the perpendicular axis.
	direction: Axis,
	/// Path of the split node this handle resizes, valid for the current
	/// frame only.
	path: NodePath,
	/// Boundary this handle moves: between children `index` and `index + 1`.
	index: usize,
	/// Window-space pointer position of the previous drag-move event; the
	/// first move of a drag establishes the baseline.
	last_position: Option<Pixels>,
}

impl SplitHandle {
	/// Thickness of the handle's interactive area, in logical pixels. The
	/// visual divider may be thinner; the wider hitbox makes the handle
	/// grabbable.
	pub(crate) const HITBOX: Pixels = Pixels(6.0);

	/// Minimum extent, in logical pixels, that a split child may be resized
	/// to by dragging. Expressed as a fraction of the parent extent when
	/// computing the drag clamp.
	pub(crate) const MIN_CHILD_EXTENT: Pixels = Pixels(120.0);

	/// The ratio a double-click resets to (even split).
	pub(crate) const RESET_RATIO: f32 = 0.5;

	/// Creates a handle for the boundary at `index` of the split at `path`.
	pub(crate) fn new(direction: Axis, path: NodePath, index: usize) -> Self {
		Self {
			direction,
			path,
			index,
			last_position: None,
		}
	}

	/// Begins a drag, clearing any stale baseline; the first drag-move event
	/// establishes the pointer baseline (the drag-start offset handed to the
	/// ghost constructor is local to the handle's hitbox, a different
	/// coordinate space than the drag-move positions, so it is unusable).
	pub(crate) fn begin_drag(&mut self) {
		self.last_position = None;
	}

	/// Applies an in-progress drag: converts the pointer delta since the
	/// previous move into a ratio delta relative to the pair extent and
	/// emits a [`SplitHandleEvent::ResizeRequested`].
	///
	/// `start_ratio` is the share of the `index` child within its pair,
	/// re-read from the layout by the owning dock area on every move so
	/// external edits during the drag are respected; `pair_extent` is the
	/// combined on-screen extent of the two children, in pixels. Both the
	/// baseline and `position` are window-space samples of the same
	/// drag-move stream, so incremental deltas stay consistent regardless of
	/// the coordinate space any individual event is reported in.
	pub(crate) fn drag_to(
		&mut self,
		position: Pixels,
		pair_extent: Pixels,
		start_ratio: f32,
		cx: &mut Context<Self>,
	) {
		if pair_extent.0 <= 0.0 {
			return;
		}
		let Some(last) = self.last_position.replace(position) else {
			return;
		};
		let delta = position.0 - last.0;
		if delta == 0.0 {
			return;
		}
		// Keep both children above MIN_CHILD_EXTENT, but never clamp harder
		// than a quarter of the pair so tiny parents stay resizable.
		let min_ratio = (Self::MIN_CHILD_EXTENT.0 / pair_extent.0).min(0.25);
		let max_ratio = 1.0 - min_ratio;
		let ratio = (start_ratio + delta / pair_extent.0).clamp(min_ratio, max_ratio);
		cx.emit(SplitHandleEvent::ResizeRequested {
			path: self.path.clone(),
			index: self.index,
			ratio,
		});
		cx.notify();
	}

	/// Ends the current drag, if any.
	pub(crate) fn end_drag(&mut self) {
		self.last_position = None;
	}

	/// Emits a [`SplitHandleEvent::ResetRequested`] for a double-click.
	pub(crate) fn reset(&mut self, cx: &mut Context<Self>) {
		cx.emit(SplitHandleEvent::ResetRequested {
			path: self.path.clone(),
			index: self.index,
		});
		cx.notify();
	}
}

impl EventEmitter<SplitHandleEvent> for SplitHandle {}

impl Render for SplitHandle {
	fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
		let colors = cx.default_colors().clone();
		let direction = self.direction;
		let handle = cx.entity();

		// Begins the drag on the handle; the ghost view is shown under the
		// pointer. The origin offset is hitbox-local and unused — the handle
		// establishes its baseline from the first drag-move event instead.
		let ghost_ctor = move |_drag: &SplitHandleDrag,
		                       _origin: Point<Pixels>,
		                       _window: &mut Window,
		                       cx: &mut App| {
			handle.update(cx, |handle, _cx| {
				handle.begin_drag();
			});
			cx.new(|_cx| SplitDragGhost { direction })
		};

		let mut root = div()
			.id(ElementId::named_usize(
				"dock-split-handle",
				path_key(&self.path),
			))
			.flex_none()
			.bg(colors.separator)
			.on_click(cx.listener(move |this, event: &ClickEvent, _window, cx| {
				if event.click_count() >= 2 {
					this.reset(cx);
				}
			}));

		// A horizontal split stacks children side by side, so its divider is
		// a vertical bar and vice versa.
		match direction {
			Axis::Horizontal => {
				root = root.w(px(Self::HITBOX.0)).h_full().cursor_col_resize();
			}
			Axis::Vertical => {
				root = root.w_full().h(px(Self::HITBOX.0)).cursor_row_resize();
			}
		}

		root.on_drag(
			SplitHandleDrag {
				path: self.path.clone(),
				index: self.index,
			},
			ghost_ctor,
		)
	}
}

/// The floating view shown under the pointer while a handle is being dragged.
struct SplitDragGhost {
	direction: Axis,
}

impl Render for SplitDragGhost {
	fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
		let colors = cx.default_colors().clone();
		let mut ghost = div().bg(colors.border).rounded_sm();
		match self.direction {
			Axis::Horizontal => ghost = ghost.w(px(2.0)).h(px(64.0)),
			Axis::Vertical => ghost = ghost.w(px(64.0)).h(px(2.0)),
		}
		ghost
	}
}
