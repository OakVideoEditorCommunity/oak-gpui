//! The timeline view: layout, interaction handling, and edit-request events.
//!
//! [`TimelineView`] is the top-level widget. It is generic over the app's
//! [`TimelineDataSource`] implementation and owns a [`TimelineState`] for
//! view-local state (zoom, scroll, playhead, selection).
//!
//! # Layout
//!
//! ```text
//! ┌──────────────────────────────────────────────────────┐
//! │ ruler (timecode ticks, work-area band, markers)      │
//! ├─────────┬────────────────────────────────────────────┤
//! │ track   │  clip area (per-track rows)                │
//! │ headers │   ┌──────┐   ┌─────────┐                   │
//! │  V1     │   │ clip │   │ clip    │        ┆ playhead │
//! │  V2     │        ┌──────────┐            ┆          │
//! │  A1     │   ┌─────────┐                 ┆          │
//! ├─────────┴────────────────────────────────────────────┤
//! │ horizontal scrollbar                                 │
//! └──────────────────────────────────────────────────────┘
//! ```
//!
//! The ruler is painted by [`TimelineRuler`](super::TimelineRuler), headers
//! by [`TrackHeader`](super::TrackHeader), clips by
//! [`ClipElement`](super::ClipElement), and the playhead by
//! [`PlayheadElement`](super::PlayheadElement). The clip area lays out one
//! row per track from the data source, sized and stacked purely from
//! [`TrackData::height`](super::TrackData::height).
//!
//! # Interactions
//!
//! * **Ruler click** — seek: moves the playhead to the pointed frame
//!   (through [`TimelineState::set_playhead`]) and emits
//!   [`TimelineEvent::PlayheadChanged`].
//! * **Clip drag** — move: horizontal motion shifts the clip in time;
//!   vertical motion across a track boundary requests a cross-track move
//!   (never onto a locked track). While dragging, the clip snaps to nearby
//!   snap points (clip edges, playhead, work-area edges, markers) when
//!   [`TimelineState::snap_enabled`]. On mouse-up the widget emits
//!   [`TimelineEvent::ClipMoveRequested`]; the host expands the request to
//!   the clip's transitive [`ClipData::linked_ids`](super::ClipData::linked_ids)
//!   group — and the widget does not change anything itself.
//! * **Clip edge drag** — trim: the ~6 px hit zones at each clip edge start
//!   a trim gesture. Emits [`TimelineEvent::ClipTrimRequested`]; the host
//!   expands the request to linked clips and adjusts `media_in` for
//!   [`TrimEdge::Start`].
//! * **Marquee** — rubber-band selection: dragging on empty clip-area space
//!   draws a selection rect; on release the hit clips go through
//!   [`TimelineState::select_range`] and [`TimelineEvent::SelectionChanged`]
//!   is emitted.
//! * **Scroll wheel** — horizontal pan; **ctrl-scroll / pinch** — zoom
//!   anchored at the cursor via [`TimelineState::set_zoom`], emitting
//!   [`TimelineEvent::ZoomChanged`].
//! * **Header separator drag** — track height resize, emitting
//!   [`TimelineEvent::TrackHeightChanged`].
//!
//! # Edit model (read this before wiring events)
//!
//! The widget **never mutates the model**. Every gesture ends in a
//! [`TimelineEvent`] describing the requested edit. The host applies it
//! through its engine and undo stack — keeping the engine the single source
//! of truth — and then calls `cx.notify()` on the data-source entity, which
//! makes the view re-read and repaint. If the engine rejects the edit
//! (locked track, collision, …), simply don't notify and the gesture has no
//! visible effect.

use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};

use crate::{
	AnyElement, App, Context, DragMoveEvent, ElementId, Entity, EventEmitter, FocusHandle,
	Focusable, Hsla, MouseButton, MouseDownEvent, PinchEvent, Pixels, Point, Render, ScrollDelta,
	ScrollWheelEvent, SharedString, Window, div, hsla, prelude::*, px,
};

use super::{
	clip::{ClipContent, ClipDecorator, ClipElement, NoopClipDecorator, TRIM_HANDLE_WIDTH},
	data::{ClipData, ClipId, TimelineDataSource, TrackData, TrackKind},
	playhead::PlayheadElement,
	ruler::{RulerMarker, TimelineRuler},
	state::TimelineState,
	time::{Frame, FrameRange, SnapKind, SnapPoint, snap},
	track_header::TrackHeader,
};

/// Which edge of a clip a trim gesture grabbed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrimEdge {
	/// The clip's left (in) edge. Trimming it changes both the clip's start
	/// and its [`ClipData::media_in`](super::ClipData::media_in).
	Start,
	/// The clip's right (out) edge.
	End,
}

/// An edit or view-state change requested by the timeline widget.
///
/// The widget emits these and then **does nothing itself**. The host
/// application applies the request through its engine and undo stack, then
/// calls `cx.notify()` on the data-source entity so the view re-reads.
///
/// # Wiring into Oak
///
/// Each variant maps onto an undoable facade call in `oakengine`:
///
/// * `ClipMoveRequested` → `engine.move_clip(clip, new_track, new_start)`
///   (ripple off), wrapped in an undo command together with the clip's
///   linked group.
/// * `ClipTrimRequested` → `engine.trim_clip(clip, edge, new_frame)`,
///   likewise grouped with linked clips.
/// * `PlayheadChanged` → `engine.set_playhead(frame)` — not undoable.
/// * `SelectionChanged` → update the app's selection state; drives the
///   inspector/effect stack panels.
/// * `TrackHeightChanged` / `ZoomChanged` → persist as per-sequence view
///   state; not undoable.
#[derive(Debug, Clone, PartialEq)]
pub enum TimelineEvent {
	/// The user asked to move a clip (and, per the gesture docs on
	/// [`TimelineView`], its linked group) to a new position.
	///
	/// `new_track` is an index into
	/// [`TimelineDataSource::track`];
	/// `new_start` is the requested [`FrameRange`](super::FrameRange) start
	/// after snapping. The engine must validate collisions and track
	/// compatibility; the widget only guarantees the source track was not
	/// locked.
	ClipMoveRequested {
		/// The clip the gesture grabbed. The host expands this to the full
		/// linked group.
		clip: ClipId,
		/// Target track index.
		new_track: usize,
		/// Requested new start frame.
		new_start: Frame,
	},

	/// The user asked to trim one edge of a clip.
	///
	/// `new_frame` is the requested new position of the grabbed `edge`,
	/// after snapping and after clamping to the neighboring clips and to
	/// zero minimum length. For [`TrimEdge::Start`] the host must also
	/// adjust `media_in` by the same delta.
	ClipTrimRequested {
		/// The trimmed clip (expand to the linked group, as with moves).
		clip: ClipId,
		/// Which edge was grabbed.
		edge: TrimEdge,
		/// Requested new frame position of that edge.
		new_frame: Frame,
	},

	/// The playhead moved, by any means (ruler seek, keyboard, playback
	/// ticker). Carries the new position. Not undoable.
	PlayheadChanged(Frame),

	/// The selection changed. The new set is readable from
	/// [`TimelineView::selection`]. Not undoable.
	SelectionChanged,

	/// A track header was clicked, toggling the track's selection state.
	///
	/// The widget keeps the selected-track set locally (readable from
	/// [`TimelineView::selected_tracks`]); the host may persist it. Not
	/// undoable.
	TrackSelected {
		/// Index of the clicked track.
		track: usize,
		/// Whether the track is now selected.
		selected: bool,
	},

	/// The user dragged a transition wedge's edge to change its length.
	///
	/// `new_length` is the requested transition duration in frames, clamped
	/// to the clip length and to a one-frame minimum. The host validates
	/// against the actual transition implementation and re-notifies.
	TransitionChanged {
		/// The clip whose transition was grabbed.
		clip: ClipId,
		/// Which edge the transition sits on.
		edge: TrimEdge,
		/// Requested new transition length.
		new_length: Frame,
	},

	/// A track's height changed via header-separator drag. The host should
	/// write this back so [`TrackData::height`](super::TrackData::height)
	/// returns it on the next read.
	TrackHeightChanged {
		/// Index of the resized track.
		track: usize,
		/// New row height.
		height: Pixels,
	},

	/// The user is dragging on the ruler, reshaping the work area. `start` /
	/// `end` are the NEW in/out frames; the widget already updated its local
	/// band (`[`TimelineState::work_area`]`), and the host should apply the
	/// new range **live** (not undoable), mirroring Olive's drag behavior.
	WorkAreaPreview {
		/// The new work-area in point.
		start: Frame,
		/// The new work-area out point.
		end: Frame,
	},

	/// The ruler work-area drag finished. The host commits the new range as
	/// ONE undoable entry whose "old" side is `old_start`/`old_end` (the
	/// range at drag start, before any live preview was applied).
	WorkAreaCommitted {
		/// The committed work-area in point.
		start: Frame,
		/// The committed work-area out point.
		end: Frame,
		/// The in point at drag start (the undo side of the command).
		old_start: Frame,
		/// The out point at drag start (the undo side of the command).
		old_end: Frame,
	},

	/// The zoom (pixels per frame) changed via ctrl-scroll or pinch.
	/// Persist as view state if desired.
	ZoomChanged(f32),
}

/// The video-editing timeline widget.
///
/// Construct with [`TimelineView::new`] over an [`Entity`] of your
/// [`TimelineDataSource`] implementation, place it in your layout like any
/// other view, and subscribe to its [`TimelineEvent`]s to apply edits:
///
/// ```ignore
/// let timeline = cx.new(|cx| TimelineView::new(model.clone(), window, cx));
/// cx.subscribe(&timeline, |this, timeline, event: &TimelineEvent, cx| {
///     match event {
///         TimelineEvent::ClipMoveRequested { clip, new_track, new_start } => {
///             this.engine.move_clip(*clip, *new_track, *new_start, cx); // undoable
///             this.model.update(cx, |_, cx| cx.notify());
///         }
///         // ...
///     }
/// }).detach();
/// ```
///
/// See the module-level docs for the layout, the full interaction list, and
/// the edit-request contract.
pub struct TimelineView<D: TimelineDataSource> {
	source: Entity<D>,
	/// View-local state (zoom, scroll, playhead, selection). Public for
	/// read access; mutate through [`TimelineState`]'s methods to preserve
	/// invariants.
	pub state: TimelineState,
	/// The set of tracks selected via their headers.
	selected_tracks: BTreeSet<usize>,
	/// Rich clip content (thumbnails / waveforms), replaced by the host.
	decorator: std::sync::Arc<std::sync::RwLock<dyn ClipDecorator>>,
	focus_handle: FocusHandle,
}

/// Width of the track-headers column, in pixels.
const HEADER_WIDTH: f32 = 160.0;

/// Minimum row height enforced by the height-resize drag, in pixels.
const MIN_TRACK_HEIGHT: f32 = 24.0;

/// Snap engagement threshold, in pixels.
const SNAP_THRESHOLD_PX: f32 = 8.0;

impl<D: TimelineDataSource> TimelineView<D> {
	/// Creates a timeline view over `source`.
	///
	/// Subscribes to `source`'s notifications: after the host applies an
	/// edit and calls `cx.notify()` on the source entity, the view re-reads
	/// all data and repaints.
	pub fn new(source: Entity<D>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
		let focus_handle = cx.focus_handle();
		cx.observe(&source, |_this, _source, cx| cx.notify())
			.detach();
		TimelineView {
			source,
			state: TimelineState::new(),
			selected_tracks: BTreeSet::new(),
			decorator: std::sync::Arc::new(std::sync::RwLock::new(NoopClipDecorator)),
			focus_handle,
		}
	}

	/// Builder: installs the host's rich-clip decorator (M12 P4 — Oak's
	/// waveform decorator). The default is the no-op decorator.
	pub fn clip_decorator(
		mut self,
		decorator: std::sync::Arc<std::sync::RwLock<dyn ClipDecorator>>,
	) -> Self {
		self.decorator = decorator;
		self
	}

	/// Replaces the clip decorator after construction (the app wires its
	/// waveform cache once the engine is up).
	pub fn set_clip_decorator(&mut self, decorator: std::sync::Arc<std::sync::RwLock<dyn ClipDecorator>>) {
		self.decorator = decorator;
	}

	/// The set of tracks selected via header clicks.
	pub fn selected_tracks(&self) -> &BTreeSet<usize> {
		&self.selected_tracks
	}

	/// Builder: sets the initial zoom (pixels per frame), clamped to
	/// [`MIN_ZOOM`](super::MIN_ZOOM)..=[`MAX_ZOOM`](super::MAX_ZOOM).
	pub fn zoom(mut self, zoom: f32) -> Self {
		self.state.set_zoom(zoom, crate::px(0.));
		self
	}

	/// Builder: enables or disables snapping initially.
	pub fn snap_enabled(mut self, enabled: bool) -> Self {
		self.state.snap_enabled = enabled;
		self
	}

	/// The data source entity this view was created over.
	pub fn source(&self) -> &Entity<D> {
		&self.source
	}

	/// The current selection, in deterministic (id) order.
	pub fn selection(&self) -> &BTreeSet<ClipId> {
		&self.state.selection
	}

	/// Seeks the playhead to `frame`, clamped to the sequence, emitting
	/// [`TimelineEvent::PlayheadChanged`] if the position changed.
	pub fn seek(&mut self, frame: Frame, cx: &mut Context<Self>) {
		let seq_len = self.sequence_length(cx);
		let old = self.state.playhead;
		self.state.set_playhead(frame, seq_len);
		if self.state.playhead != old {
			cx.emit(TimelineEvent::PlayheadChanged(self.state.playhead));
			cx.notify();
		}
	}

	/// The sequence length from the data source.
	fn sequence_length(&self, cx: &mut Context<Self>) -> Frame {
		self.source.read(cx).sequence_length()
	}

	/// The current range of the clip with `id`, scanned from the data source.
	/// Falls back to an empty range when the clip no longer exists.
	fn clip_range(&self, id: ClipId, cx: &mut Context<Self>) -> FrameRange {
		let source = self.source.read(cx);
		for index in 0..source.track_count() {
			if let Some(track) = source.track(index) {
				for clip in track.clips() {
					if clip.id() == id {
						return clip.range();
					}
				}
			}
		}
		FrameRange::new(Frame::ZERO, Frame::ZERO)
	}

	/// Snap points gathered fresh from the data source: work-area edges,
	/// markers, the playhead, and every enabled clip edge. `dragged` (the
	/// clip being moved) is excluded so a clip never snaps to itself.
	fn snap_points(&self, dragged: Option<ClipId>, cx: &mut Context<Self>) -> Vec<SnapPoint> {
		let source = self.source.read(cx);
		let mut points = Vec::new();
		if let Some(area) = self.state.work_area {
			points.push(SnapPoint {
				frame: area.start,
				kind: SnapKind::WorkAreaEdge,
			});
			points.push(SnapPoint {
				frame: area.end,
				kind: SnapKind::WorkAreaEdge,
			});
		}
		for marker in source.markers() {
			points.push(SnapPoint {
				frame: marker.frame,
				kind: SnapKind::Marker,
			});
		}
		points.push(SnapPoint {
			frame: self.state.playhead,
			kind: SnapKind::Playhead,
		});
		for index in 0..source.track_count() {
			if let Some(track) = source.track(index) {
				for clip in track.clips() {
					if clip.is_enabled() && Some(clip.id()) != dragged {
						let range = clip.range();
						points.push(SnapPoint {
							frame: range.start,
							kind: SnapKind::ClipStart,
						});
						points.push(SnapPoint {
							frame: range.end,
							kind: SnapKind::ClipEnd,
						});
					}
				}
			}
		}
		points
	}

	/// Index of the track whose row contains screen `y` (relative to the
	/// clip area's top). Falls back to the last track when below all rows.
	fn track_at_y(&self, y: f32, cx: &mut Context<Self>) -> usize {
		let source = self.source.read(cx);
		let count = source.track_count();
		let mut acc = 0.0;
		for index in 0..count {
			if let Some(track) = source.track(index) {
				acc += track.height().0.max(MIN_TRACK_HEIGHT);
				if y < acc {
					return index;
				}
			}
		}
		count.saturating_sub(1)
	}

	/// Whether the track at `index` is locked. Out-of-range tracks count as
	/// locked so drop requests onto them are rejected.
	fn track_locked(&self, index: usize, cx: &mut Context<Self>) -> bool {
		self.source
			.read(cx)
			.track(index)
			.map(|track| track.is_locked())
			.unwrap_or(true)
	}

	/// Updates a clip-move drag from the pointer position: computes the new
	/// start frame (with snapping) and the track under the cursor.
	fn update_clip_drag(
		&mut self,
		event: &DragMoveEvent<Arc<RwLock<ClipDrag>>>,
		cx: &mut Context<Self>,
	) {
		let drag = Arc::clone(event.drag(cx));
		let press = cx
			.active_drag
			.as_ref()
			.map(|drag| drag.cursor_offset)
			.unwrap_or_default();
		let now = event.event.position - event.bounds.origin;
		let mut drag = drag.write().expect("clip drag lock is not poisoned");
		let wrapper_x = self.state.point_at_frame(drag.original_start).0;
		let dx = now.x.0 - (wrapper_x + press.x.0);
		let mut new_start = Frame(drag.original_start.0 + (dx / self.state.zoom).round() as i64);
		let new_track = self.track_at_y(now.y.0, cx);
		if self.state.snap_enabled {
			if let Some(result) = snap(
				new_start,
				self.snap_points(Some(drag.clip), cx).into_iter(),
				px(SNAP_THRESHOLD_PX),
				self.state.zoom,
			) {
				new_start = result.frame;
			}
		}
		drag.new_start = new_start;
		drag.new_track = new_track;
	}

	/// Updates a trim drag from the pointer position: computes the new
	/// position of the grabbed edge, clamped to keep the clip non-empty and
	/// inside the sequence, then snapped and re-clamped.
	fn update_trim_drag(
		&mut self,
		event: &DragMoveEvent<Arc<RwLock<TrimDrag>>>,
		cx: &mut Context<Self>,
	) {
		let drag = Arc::clone(event.drag(cx));
		let press = cx
			.active_drag
			.as_ref()
			.map(|drag| drag.cursor_offset)
			.unwrap_or_default();
		let now = event.event.position - event.bounds.origin;
		let seq_len = self.sequence_length(cx);
		let mut drag = drag.write().expect("trim drag lock is not poisoned");
		let range = self.clip_range(drag.clip, cx);
		let handle_x = match drag.edge {
			TrimEdge::Start => now.x.0 - press.x.0,
			TrimEdge::End => now.x.0 - press.x.0 + TRIM_HANDLE_WIDTH,
		};
		let mut new_frame = self.state.frame_at_point(px(handle_x));
		let clamp = |frame: Frame| match drag.edge {
			TrimEdge::Start => frame.max(Frame::ZERO).min(Frame((range.end.0 - 1).max(0))),
			TrimEdge::End => frame
				.max(Frame((range.start.0 + 1).min(seq_len.0)))
				.min(seq_len),
		};
		new_frame = clamp(new_frame);
		if self.state.snap_enabled {
			if let Some(result) = snap(
				new_frame,
				self.snap_points(Some(drag.clip), cx).into_iter(),
				px(SNAP_THRESHOLD_PX),
				self.state.zoom,
			) {
				new_frame = clamp(result.frame);
			}
		}
		drag.new_frame = new_frame;
	}

	/// Updates a track-height drag from the pointer position.
	fn update_height_drag(
		&mut self,
		event: &DragMoveEvent<Arc<RwLock<HeightDrag>>>,
		cx: &mut Context<Self>,
	) {
		let drag = Arc::clone(event.drag(cx));
		let press = cx
			.active_drag
			.as_ref()
			.map(|drag| drag.cursor_offset)
			.unwrap_or_default();
		let now = event.event.position - event.bounds.origin;
		let mut drag = drag.write().expect("height drag lock is not poisoned");
		let dy = now.y.0 - (press.y.0 + drag.separator_y);
		drag.new_height = px((drag.start_height.0 + dy).max(MIN_TRACK_HEIGHT));
	}

	/// Updates a marquee selection from the pointer position: hit-tests the
	/// rows intersecting the rubber-band rect and selects the clips inside.
	fn update_marquee(
		&mut self,
		event: &DragMoveEvent<MarqueeDrag>,
		rows: &[RowData],
		cx: &mut Context<Self>,
	) {
		let press = cx
			.active_drag
			.as_ref()
			.map(|drag| drag.cursor_offset)
			.unwrap_or_default();
		let now = event.event.position - event.bounds.origin;
		let mut frame_start = self.state.frame_at_point(px(press.x.0));
		let mut frame_end = self.state.frame_at_point(px(now.x.0));
		if frame_start > frame_end {
			std::mem::swap(&mut frame_start, &mut frame_end);
		}
		let y0 = press.y.min(now.y).0;
		let y1 = press.y.max(now.y).0;
		let mut ids = BTreeSet::new();
		for row in rows {
			if y1 >= row.y && y0 <= row.y + row.height {
				for clip in &row.clips {
					if clip.enabled
						&& clip.range.end.0 > frame_start.0
						&& clip.range.start.0 < frame_end.0
					{
						ids.insert(clip.id);
					}
				}
			}
		}
		self.state.select_range(ids);
	}

	/// Emits [`TimelineEvent::ClipMoveRequested`] for a finished clip move,
	/// unless the gesture didn't move the clip or a locked track was involved.
	fn finish_clip_drag(&mut self, drag: &Arc<RwLock<ClipDrag>>, cx: &mut Context<Self>) {
		let (clip, original_start, original_track, new_start, new_track) = {
			let drag = drag.read().expect("clip drag lock is not poisoned");
			(
				drag.clip,
				drag.original_start,
				drag.original_track,
				drag.new_start,
				drag.new_track,
			)
		};
		if (new_start, new_track) != (original_start, original_track)
			&& !self.track_locked(original_track, cx)
			&& !self.track_locked(new_track, cx)
		{
			cx.emit(TimelineEvent::ClipMoveRequested {
				clip,
				new_track,
				new_start,
			});
			cx.notify();
		}
	}

	/// Emits [`TimelineEvent::ClipTrimRequested`] for a finished trim.
	fn finish_trim_drag(&mut self, drag: &Arc<RwLock<TrimDrag>>, cx: &mut Context<Self>) {
		let (clip, edge, original_frame, new_frame) = {
			let drag = drag.read().expect("trim drag lock is not poisoned");
			(drag.clip, drag.edge, drag.original_frame, drag.new_frame)
		};
		if new_frame != original_frame {
			cx.emit(TimelineEvent::ClipTrimRequested {
				clip,
				edge,
				new_frame,
			});
			cx.notify();
		}
	}

	/// Apply a transition-resize drag move: scale the requested length by the
	/// horizontal mouse delta and clamp to the clip length.
	fn update_transition_drag(
		&mut self,
		event: &DragMoveEvent<Arc<RwLock<TransitionDrag>>>,
		cx: &mut Context<Self>,
	) {
		let drag = Arc::clone(event.drag(cx));
		let press = cx
			.active_drag
			.as_ref()
			.map(|drag| drag.cursor_offset)
			.unwrap_or_default();
		let now = event.event.position - event.bounds.origin;
		let dx = now.x - press.x;
		let clip_len = {
			let range = self.clip_range(drag.read().unwrap().clip, cx);
			range.end - range.start
		};
		let mut drag = drag.write().expect("transition drag lock is not poisoned");
		let frames_per_pixel = 1.0 / self.state.zoom as f64;
		let delta = (dx.0 as f64 * frames_per_pixel).round() as i64;
		let requested = drag.original_length.0 + delta;
		drag.new_length = Frame(requested.clamp(1, clip_len.0.max(1)));
		cx.notify();
	}

	/// Emits [`TimelineEvent::TransitionChanged`] for a finished resize.
	fn finish_transition_drag(
		&mut self,
		drag: &Arc<RwLock<TransitionDrag>>,
		cx: &mut Context<Self>,
	) {
		let (clip, edge, original_length, new_length) = {
			let drag = drag.read().expect("transition drag lock is not poisoned");
			(drag.clip, drag.edge, drag.original_length, drag.new_length)
		};
		if new_length != original_length {
			cx.emit(TimelineEvent::TransitionChanged {
				clip,
				edge,
				new_length,
			});
			cx.notify();
		}
	}

	/// Emits [`TimelineEvent::TrackHeightChanged`] for a finished resize.
	fn finish_height_drag(&mut self, drag: &Arc<RwLock<HeightDrag>>, cx: &mut Context<Self>) {
		let (track, start_height, new_height) = {
			let drag = drag.read().expect("height drag lock is not poisoned");
			(drag.track, drag.start_height, drag.new_height)
		};
		if new_height.0 != start_height.0 {
			cx.emit(TimelineEvent::TrackHeightChanged {
				track,
				height: new_height,
			});
			cx.notify();
		}
	}

	/// How close (in pixels) a press must be to a work-area band edge for the
	/// drag to resize that edge instead of reshaping the whole band.
	const WORK_AREA_EDGE_THRESHOLD_PX: f32 = 6.0;

	/// Updates a ruler work-area drag from the pointer position.
	///
	/// The first drag that passes the click/drag threshold (see
	/// [`RulerDrag`]) reshapes the work area: pressing near an existing band
	/// edge moves that edge, any other press replaces the whole band with the
	/// range spanned from the press point. The widget updates its local band
	/// immediately and emits [`TimelineEvent::WorkAreaPreview`] so the host
	/// can apply the range live (not undoable). A plain click never reaches
	/// here — it only seeks (the ruler's `on_mouse_down`).
	fn update_ruler_drag(
		&mut self,
		event: &DragMoveEvent<Arc<RwLock<RulerDrag>>>,
		cx: &mut Context<Self>,
	) {
		let drag = Arc::clone(event.drag(cx));
		let press = cx
			.active_drag
			.as_ref()
			.map(|drag| drag.cursor_offset)
			.unwrap_or_default();
		let now = event.event.position - event.bounds.origin;
		let mut drag = drag.write().expect("ruler drag lock is not poisoned");
		let dx = (now.x.0 - press.x.0).abs();
		if dx < 3.0 {
			// Still within the click threshold; keep seeking-only behavior.
			return;
		}
		if !drag.resizing {
			// First real drag movement: pick what the gesture reshapes. A
			// press within `WORK_AREA_EDGE_THRESHOLD_PX` of an existing band
			// edge moves that edge; otherwise the whole band is replaced.
			let edge = match self.state.work_area {
				Some(band) => {
					let in_edge = self.state.point_at_frame(band.start).0;
					let out_edge = self.state.point_at_frame(band.end).0;
					if (press.x.0 - in_edge).abs() <= Self::WORK_AREA_EDGE_THRESHOLD_PX {
						EdgeKind::In
					} else if (press.x.0 - out_edge).abs() <= Self::WORK_AREA_EDGE_THRESHOLD_PX {
						EdgeKind::Out
					} else {
						EdgeKind::Whole
					}
				}
				None => EdgeKind::Whole,
			};
			drag.edge = Some(edge);
			drag.old_band = self.state.work_area;
			drag.resizing = true;
		}
		let Some(edge) = drag.edge else {
			return;
		};
		let seq_len = self.sequence_length(cx).0.max(1);
		let frame = self.state.frame_at_point(now.x).0.clamp(0, seq_len);
		let press_frame = self.state.frame_at_point(press.x).0.clamp(0, seq_len);
		let band = reshape_work_area(drag.old_band, Frame(press_frame), Frame(frame), edge, seq_len);
		self.state.work_area = Some(band);
		drag.new_band = Some(band);
		cx.emit(TimelineEvent::WorkAreaPreview {
			start: band.start,
			end: band.end,
		});
		cx.notify();
	}

	/// Emits [`TimelineEvent::WorkAreaCommitted`] for a finished ruler drag:
	/// the host commits the new range as ONE undoable entry whose old side is
	/// the band at drag start (`0..0` when the drag created a new band).
	fn finish_ruler_drag(&mut self, drag: &Arc<RwLock<RulerDrag>>, cx: &mut Context<Self>) {
		let (resizing, old_band, new_band) = {
			let drag = drag.read().expect("ruler drag lock is not poisoned");
			(drag.resizing, drag.old_band, drag.new_band)
		};
		if !resizing {
			return;
		}
		let Some(new) = new_band else {
			return;
		};
		// A drag that created a new band (no old one) reports the empty
		// `0..0` range as its old side; the host's undoable commit restores
		// disabled + the empty range on undo.
		let old = old_band.unwrap_or(FrameRange::new(Frame::ZERO, Frame::ZERO));
		if old != new {
			cx.emit(TimelineEvent::WorkAreaCommitted {
				start: new.start,
				end: new.end,
				old_start: old.start,
				old_end: old.end,
			});
			cx.notify();
		}
	}

	/// Emits [`TimelineEvent::SelectionChanged`] for a finished marquee.
	fn finish_marquee(&mut self, cx: &mut Context<Self>) {
		cx.emit(TimelineEvent::SelectionChanged);
		cx.notify();
	}

	// --- interaction handlers (private) ---
	//
	// The gesture logic lives in the render method's inline listeners and
	// the `update_*` / `finish_*` helpers above; these signatures are
	// retained (with `#[allow(dead_code)]`) as documentation of the
	// gesture set the view wires up.

	#[allow(dead_code)] // gesture documentation; see the section comment above
	fn on_ruler_mouse_down(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

	#[allow(dead_code)] // gesture documentation; see the section comment above
	fn on_clip_drag_move(&mut self, _clip: ClipId, _window: &mut Window, _cx: &mut Context<Self>) {}

	#[allow(dead_code)] // gesture documentation; see the section comment above
	fn on_clip_drag_drop(&mut self, _clip: ClipId, _window: &mut Window, _cx: &mut Context<Self>) {}

	#[allow(dead_code)] // gesture documentation; see the section comment above
	fn on_trim_drag(
		&mut self,
		_clip: ClipId,
		_edge: TrimEdge,
		_window: &mut Window,
		_cx: &mut Context<Self>,
	) {
	}

	#[allow(dead_code)] // gesture documentation; see the section comment above
	fn on_marquee(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

	#[allow(dead_code)] // gesture documentation; see the section comment above
	fn on_scroll(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}
}

impl<D: TimelineDataSource> Focusable for TimelineView<D> {
	fn focus_handle(&self, _cx: &App) -> FocusHandle {
		self.focus_handle.clone()
	}
}

impl<D: TimelineDataSource> EventEmitter<TimelineEvent> for TimelineView<D> {}

impl<D: TimelineDataSource> Render for TimelineView<D> {
	fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
		let state = self.state.clone();
		let source = self.source.read(cx);
		let frame_rate = source.frame_rate();
		let seq_len = source.sequence_length();

		// Snapshot the rows: one per track, stacked from the top of the clip
		// area, with y positions accumulated from the model's track heights.
		let mut rows: Vec<RowData> = Vec::new();
		let mut y = 0.0;
		for index in 0..source.track_count() {
			if let Some(track) = source.track(index) {
				let height = track.height().0.max(MIN_TRACK_HEIGHT);
				let clips = track
					.clips()
					.iter()
					.map(|clip| {
						let color = clip.color().unwrap_or_else(|| kind_color(track.kind()));
						let in_transition = clip
							.in_transition()
							.filter(|duration| duration.0 > 0)
							.map(|duration| FrameRange::new(Frame::ZERO, duration));
						let out_transition = clip
							.out_transition()
							.filter(|duration| duration.0 > 0)
							.map(|duration| FrameRange::new(Frame::ZERO, duration));
						ClipRenderData {
							id: clip.id(),
							range: clip.range(),
							label: clip.label(),
							color,
							enabled: clip.is_enabled(),
							in_transition,
							out_transition,
						}
					})
					.collect();
				rows.push(RowData {
					index,
					name: track.name(),
					kind: track.kind(),
					height,
					y,
					locked: track.is_locked(),
					muted: track.is_muted(),
					solo: track.is_solo(),
					visible: track.is_visible(),
					clips,
				});
				y += height;
			}
		}

		// The marquee handler needs the row geometry; keep a snapshot for it
		// (the clip-area child iterator consumes `rows` below).
		let marquee_rows = Arc::new(rows.clone());
		let playhead_x = state.point_at_frame(state.playhead).0;
		let decorator = self.decorator.clone();

		// Sequence markers → ruler paint data (the data source provides them
		// sorted; the ruler colors them, falling back to the accent).
		let marker_color = marker_accent_color();
		let markers: Vec<RulerMarker> = source
			.markers()
			.into_iter()
			.map(|m| RulerMarker {
				frame: m.frame,
				color: m.color.unwrap_or(marker_color),
			})
			.collect();

		let ruler = div()
			.flex()
			.flex_row()
			.h(px(32.))
			.flex_shrink_0()
			.child(div().w(px(HEADER_WIDTH)).flex_shrink_0())
			.child(
				div()
					.flex_1()
					.h_full()
					.id("timeline-ruler")
					.on_mouse_down(
						MouseButton::Left,
						cx.listener(|this, event: &MouseDownEvent, _window, cx| {
							// The ruler's left edge aligns with the clip
							// area's, i.e. HEADER_WIDTH px from the window's
							// left edge (this widget is expected to sit at
							// window x = 0).
							let frame = this
								.state
								.frame_at_point(event.position.x - px(HEADER_WIDTH));
							this.seek(frame, cx);
						}),
					)
					.on_drag(
						Arc::new(RwLock::new(RulerDrag {
							old_band: None,
							new_band: None,
							edge: None,
							resizing: false,
						})),
						drag_ghost,
					)
					.on_drag_move(cx.listener(
						|this, event: &DragMoveEvent<Arc<RwLock<RulerDrag>>>, _window, cx| {
							this.update_ruler_drag(event, cx);
						},
					))
					.on_drop(
						cx.listener(|this, drag: &Arc<RwLock<RulerDrag>>, _window, cx| {
							this.finish_ruler_drag(drag, cx);
						}),
					)
					.child(
						TimelineRuler::new(state.clone(), frame_rate, seq_len)
							.markers(markers),
					),
			);

		let headers = div()
			.w(px(HEADER_WIDTH))
			.flex_shrink_0()
			.flex()
			.flex_col()
			.id("timeline-track-headers")
			.on_drag_move(cx.listener(
				|this, event: &DragMoveEvent<Arc<RwLock<HeightDrag>>>, _window, cx| {
					this.update_height_drag(event, cx);
				},
			))
			.on_drop(
				cx.listener(|this, drag: &Arc<RwLock<HeightDrag>>, _window, cx| {
					this.finish_height_drag(drag, cx);
				}),
			)
			.on_drag_move(cx.listener(
				|this, event: &DragMoveEvent<Arc<RwLock<TransitionDrag>>>, _window, cx| {
					this.update_transition_drag(event, cx);
				},
			))
			.on_drop(
				cx.listener(|this, drag: &Arc<RwLock<TransitionDrag>>, _window, cx| {
					this.finish_transition_drag(drag, cx);
				}),
			)
			.children(rows.iter().map(|row| {
				let height = row.height;
				let separator_y = row.y + height - TrackHeader::SEPARATOR_HEIGHT;
				div()
					.h(px(height))
					.relative()
					.child(
						div()
							.id(ElementId::named_usize("timeline-track-header", row.index))
							.h(px(height - TrackHeader::SEPARATOR_HEIGHT))
							.cursor_pointer()
							.on_click({
								let track = row.index;
								cx.listener(move |this, _event: &crate::ClickEvent, _window, cx| {
									let selected = if this.selected_tracks.contains(&track) {
										this.selected_tracks.remove(&track);
										false
									} else {
										this.selected_tracks.insert(track);
										true
									};
									cx.emit(TimelineEvent::TrackSelected { track, selected });
									cx.notify();
								})
							})
							.child(
								TrackHeader::new(row.index, row.name.clone(), row.kind)
									.locked(row.locked)
									.muted(row.muted)
									.solo(row.solo)
									.visible(row.visible),
							),
					)
					.child(
						div()
							.absolute()
							.left(px(0.))
							.right(px(0.))
							.bottom(px(0.))
							.h(px(TrackHeader::SEPARATOR_HEIGHT))
							.id(ElementId::named_usize("timeline-track-resize", row.index))
							.cursor_row_resize()
							.on_drag(
								Arc::new(RwLock::new(HeightDrag {
									track: row.index,
									start_height: px(height),
									new_height: px(height),
									separator_y,
								})),
								drag_ghost,
							),
					)
			}));

		let clip_area = div()
			.flex_1()
			.h_full()
			.id("timeline-clip-area")
			.relative()
			.overflow_hidden()
			.flex()
			.flex_col()
			.on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _window, cx| {
				if event.modifiers.control || event.modifiers.platform {
					let anchor = event.position.x - px(HEADER_WIDTH);
					let factor = match event.delta {
						ScrollDelta::Pixels(delta) => 1.0 + delta.y.0 * 0.01,
						ScrollDelta::Lines(delta) => 1.0 + delta.y * 0.01,
					}
					.clamp(0.5, 2.0);
					let old = this.state.zoom;
					this.state.set_zoom(old * factor, anchor);
					if (this.state.zoom - old).abs() > f32::EPSILON {
						cx.emit(TimelineEvent::ZoomChanged(this.state.zoom));
					}
				} else {
					let dx = match event.delta {
						ScrollDelta::Pixels(delta) => delta.y.0,
						ScrollDelta::Lines(delta) => delta.y * 24.0,
					};
					this.state.scroll_offset.x = px((this.state.scroll_offset.x.0 + dx).max(0.0));
				}
				cx.notify();
			}))
			.on_pinch(cx.listener(|this, event: &PinchEvent, _window, cx| {
				let anchor = event.position.x - px(HEADER_WIDTH);
				let old = this.state.zoom;
				this.state.set_zoom(old * (1.0 + event.delta), anchor);
				if (this.state.zoom - old).abs() > f32::EPSILON {
					cx.emit(TimelineEvent::ZoomChanged(this.state.zoom));
				}
				cx.notify();
			}))
			.on_drag(MarqueeDrag, drag_ghost)
			.on_drag_move(cx.listener(
				|this, event: &DragMoveEvent<Arc<RwLock<ClipDrag>>>, _window, cx| {
					this.update_clip_drag(event, cx);
				},
			))
			.on_drag_move(cx.listener(
				|this, event: &DragMoveEvent<Arc<RwLock<TrimDrag>>>, _window, cx| {
					this.update_trim_drag(event, cx);
				},
			))
			.on_drag_move({
				cx.listener(
					move |this, event: &DragMoveEvent<MarqueeDrag>, _window, cx| {
						this.update_marquee(event, marquee_rows.as_slice(), cx);
					},
				)
			})
			.on_drop(
				cx.listener(|this, drag: &Arc<RwLock<ClipDrag>>, _window, cx| {
					this.finish_clip_drag(drag, cx);
				}),
			)
			.on_drop(
				cx.listener(|this, drag: &Arc<RwLock<TrimDrag>>, _window, cx| {
					this.finish_trim_drag(drag, cx);
				}),
			)
			.on_drop(cx.listener(|this, _drag: &MarqueeDrag, _window, cx| {
				this.finish_marquee(cx);
			}))
			.on_drop(
				cx.listener(|this, drag: &Arc<RwLock<HeightDrag>>, _window, cx| {
					this.finish_height_drag(drag, cx);
				}),
			)
			.children(rows.into_iter().map(move |row| {
				let height = row.height;
				let row_locked = row.locked;
				let kind = row.kind;
				let row_index = row.index;
				let state = &state;
				let decorator = &decorator;
				div()
					.h(px(height))
					.relative()
					.children(row.clips.into_iter().map(move |clip| {
						let x0 = state.point_at_frame(clip.range.start).0;
						let x1 = state.point_at_frame(clip.range.end).0;
						let width = (x1 - x0).max(1.0);
						let clip_height = (height - TrackHeader::SEPARATOR_HEIGHT).max(1.0);
						let mut children: Vec<AnyElement> = vec![
							ClipElement::new(
								clip.id,
								clip.label.clone(),
								clip.color,
								clip.in_transition,
								clip.out_transition,
								decorator.clone(),
							)
							.selected(state.is_selected(clip.id))
							.enabled(clip.enabled)
							.locked(row_locked)
							.content(match kind {
								TrackKind::Video => ClipContent::Thumbnails,
								TrackKind::Audio => ClipContent::Waveform,
								TrackKind::Subtitle => ClipContent::None,
							})
							.into_any_element(),
						];
						if !row_locked {
							children.push(
								div()
									.absolute()
									.left(px(0.))
									.top(px(0.))
									.bottom(px(0.))
									.w(px(TRIM_HANDLE_WIDTH))
									.id(ElementId::named_usize(
										"timeline-trim-start",
										clip.id.0 as usize,
									))
									.cursor_ew_resize()
									.on_drag(
										Arc::new(RwLock::new(TrimDrag {
											clip: clip.id,
											edge: TrimEdge::Start,
											original_frame: clip.range.start,
											new_frame: clip.range.start,
										})),
										drag_ghost,
									)
									.into_any_element(),
							);
							children.push(
								div()
									.absolute()
									.right(px(0.))
									.top(px(0.))
									.bottom(px(0.))
									.w(px(TRIM_HANDLE_WIDTH))
									.id(ElementId::named_usize(
										"timeline-trim-end",
										clip.id.0 as usize,
									))
									.cursor_ew_resize()
									.on_drag(
										Arc::new(RwLock::new(TrimDrag {
											clip: clip.id,
											edge: TrimEdge::End,
											original_frame: clip.range.end,
											new_frame: clip.range.end,
										})),
										drag_ghost,
									)
									.into_any_element(),
							);
						}

						// Transition-resize handles sit at the wedges' edges.
						if let Some(transition) = clip.in_transition {
							children.push(
								div()
									.absolute()
									.left(px(0.))
									.top(px(0.))
									.bottom(px(0.))
									.w(px(TRIM_HANDLE_WIDTH))
									.id(ElementId::named_usize(
										"timeline-transition-handle",
										clip.id.0 as usize,
									))
									.cursor_ew_resize()
									.on_drag(
										Arc::new(RwLock::new(TransitionDrag {
											clip: clip.id,
											edge: TrimEdge::Start,
											original_length: transition.end,
											new_length: transition.end,
										})),
										drag_ghost,
									)
									.into_any_element(),
							);
						}
						if let Some(transition) = clip.out_transition {
							children.push(
								div()
									.absolute()
									.right(px(0.))
									.top(px(0.))
									.bottom(px(0.))
									.w(px(TRIM_HANDLE_WIDTH))
									.id(ElementId::named_usize(
										"timeline-transition-handle",
										clip.id.0 as usize,
									))
									.cursor_ew_resize()
									.on_drag(
										Arc::new(RwLock::new(TransitionDrag {
											clip: clip.id,
											edge: TrimEdge::End,
											original_length: transition.end,
											new_length: transition.end,
										})),
										drag_ghost,
									)
									.into_any_element(),
							);
						}
						div()
							.absolute()
							.left(px(x0))
							// Slight vertical inset: the design's clips read
							// as rounded bars with a small gap between rows,
							// not full-height slabs. The corners are rounded
							// in ClipElement's painted quads (content masks
							// are rectangular only).
							.top(px(2.))
							.w(px(width))
							.h(px((clip_height - 4.0).max(1.0)))
							.overflow_hidden()
							.id(ElementId::named_usize("timeline-clip", clip.id.0 as usize))
							.on_drag(
								Arc::new(RwLock::new(ClipDrag {
									clip: clip.id,
									original_start: clip.range.start,
									original_track: row_index,
									new_start: clip.range.start,
									new_track: row_index,
								})),
								drag_ghost,
							)
							.children(children)
					}))
			}));

		let playhead = div()
			.absolute()
			.left(px(HEADER_WIDTH + playhead_x))
			.top(px(0.))
			.bottom(px(0.))
			.w(px(1.))
			.child(PlayheadElement::new(px(0.), playhead_color()));

		div().size_full().flex().flex_col().child(ruler).child(
			div()
				.flex()
				.flex_row()
				.flex_1()
				.relative()
				.child(headers)
				.child(clip_area)
				.child(playhead),
		)
	}
}

/// Which edge of a work-area band a ruler drag reshapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EdgeKind {
	/// The band's in (left) edge.
	In,
	/// The band's out (right) edge.
	Out,
	/// The whole band, replaced by the press-to-cursor range.
	Whole,
}

/// Shared state for a ruler work-area drag. The drag starts on every ruler
/// press (alongside the seek), but only starts reshaping once the pointer
/// moves past the click threshold.
struct RulerDrag {
	/// The work-area band at drag start; `None` when the drag creates a new
	/// band.
	old_band: Option<FrameRange>,
	/// The band as the pointer moved.
	new_band: Option<FrameRange>,
	/// What the gesture reshapes, decided on the first real movement.
	edge: Option<EdgeKind>,
	/// Whether the pointer has passed the click threshold.
	resizing: bool,
}

/// Shared state for a clip-move gesture (the clip wrapper starts it, the
/// clip area updates and finishes it).
struct ClipDrag {
	clip: ClipId,
	original_start: Frame,
	original_track: usize,
	new_start: Frame,
	new_track: usize,
}

/// Shared state for a trim gesture; see [`TrimEdge`].
struct TrimDrag {
	clip: ClipId,
	edge: TrimEdge,
	original_frame: Frame,
	new_frame: Frame,
}

/// In-flight payload of a transition-resize drag.
struct TransitionDrag {
	clip: ClipId,
	edge: TrimEdge,
	original_length: Frame,
	new_length: Frame,
}

/// Shared state for a track-height resize gesture.
struct HeightDrag {
	track: usize,
	start_height: Pixels,
	new_height: Pixels,
	/// The separator strip's top edge, relative to the headers column's top.
	separator_y: f32,
}

/// Marker type for a marquee (rubber-band) selection gesture.
struct MarqueeDrag;

/// Layout snapshot of one track row, captured during render.
#[derive(Clone)]
struct RowData {
	index: usize,
	name: SharedString,
	kind: TrackKind,
	height: f32,
	y: f32,
	locked: bool,
	muted: bool,
	solo: bool,
	visible: bool,
	clips: Vec<ClipRenderData>,
}

/// Layout snapshot of one clip, captured during render.
#[derive(Clone)]
struct ClipRenderData {
	id: ClipId,
	range: FrameRange,
	label: SharedString,
	color: Hsla,
	enabled: bool,
	in_transition: Option<FrameRange>,
	out_transition: Option<FrameRange>,
}

/// The ghost rendered under the cursor during any timeline drag.
struct DragPreview;

impl Render for DragPreview {
	fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
		div().size_full().bg(hsla(0.6, 0.7, 0.9, 0.35))
	}
}

/// Builds the drag ghost view for a drag of any value type.
fn drag_ghost<T>(
	_drag: &T,
	_offset: Point<Pixels>,
	_window: &mut Window,
	cx: &mut App,
) -> Entity<DragPreview> {
	cx.new(|_cx| DragPreview)
}

/// The playhead line color.
fn playhead_color() -> Hsla {
	hsla(0.0, 0.0, 0.9, 0.9)
}

/// Reshapes the work-area band for a ruler drag move.
///
/// * `band` — the band at drag start (`None` = the drag creates a new one).
/// * `press_frame` / `current_frame` — the clamped press and pointer frames.
/// * `edge` — what the gesture reshapes (see [`EdgeKind`]).
///
/// `In`/`Out` keep at least one frame of band length; `Whole` replaces the
/// band with the press-to-pointer range, always `[start, end)` with
/// `end > start`.
fn reshape_work_area(
	band: Option<FrameRange>,
	press_frame: Frame,
	current_frame: Frame,
	edge: EdgeKind,
	seq_len: i64,
) -> FrameRange {
	let mut band = band.unwrap_or_else(|| FrameRange::new(Frame::ZERO, Frame(seq_len)));
	match edge {
		EdgeKind::In => band.start = Frame(current_frame.0.min((band.end.0 - 1).max(0))),
		EdgeKind::Out => band.end = Frame(current_frame.0.max(band.start.0 + 1)),
		EdgeKind::Whole => {
			band = if press_frame.0 <= current_frame.0 {
				FrameRange::new(
					press_frame,
					Frame(current_frame.0.max(press_frame.0 + 1)),
				)
			} else {
				FrameRange::new(current_frame, Frame(press_frame.0.max(current_frame.0 + 1)))
			};
		}
	}
	band
}

/// The default marker color (used when a marker carries no explicit color):
/// the theme-agnostic amber accent, distinct from the playhead and the
/// work-area band.
fn marker_accent_color() -> Hsla {
	hsla(0.10, 0.85, 0.55, 1.0)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reshape_work_area_moves_edges_within_bounds() {
		let band = FrameRange::new(Frame(20), Frame(80));
		// In edge: clamped so the band never collapses or inverts.
		let reshaped = reshape_work_area(Some(band), Frame(20), Frame(50), EdgeKind::In, 1000);
		assert_eq!(reshaped, FrameRange::new(Frame(50), Frame(80)));
		let reshaped = reshape_work_area(Some(band), Frame(20), Frame(95), EdgeKind::In, 1000);
		assert_eq!(reshaped, FrameRange::new(Frame(79), Frame(80)));
		// Out edge.
		let reshaped = reshape_work_area(Some(band), Frame(80), Frame(40), EdgeKind::Out, 1000);
		assert_eq!(reshaped, FrameRange::new(Frame(20), Frame(40)));
		let reshaped = reshape_work_area(Some(band), Frame(80), Frame(10), EdgeKind::Out, 1000);
		assert_eq!(reshaped, FrameRange::new(Frame(20), Frame(21)));
	}

	#[test]
	fn reshape_work_area_whole_replaces_or_creates_the_band() {
		// Forward drag: press-to-cursor range.
		let reshaped = reshape_work_area(Some(FrameRange::new(Frame(0), Frame(10))), Frame(30), Frame(70), EdgeKind::Whole, 1000);
		assert_eq!(reshaped, FrameRange::new(Frame(30), Frame(70)));
		// Backward drag: the range is normalized (start <= end).
		let reshaped = reshape_work_area(Some(FrameRange::new(Frame(0), Frame(10))), Frame(70), Frame(30), EdgeKind::Whole, 1000);
		assert_eq!(reshaped, FrameRange::new(Frame(30), Frame(70)));
		// Creating a band from nothing uses the virtual 0..length band as the
		// base for edge moves.
		let reshaped = reshape_work_area(None, Frame(0), Frame(40), EdgeKind::Out, 1000);
		assert_eq!(reshaped, FrameRange::new(Frame(0), Frame(40)));
	}
}

/// The default body color for clips on a track of `kind`.
fn kind_color(kind: TrackKind) -> Hsla {
	match kind {
		TrackKind::Video => hsla(0.58, 0.45, 0.35, 1.0),
		TrackKind::Audio => hsla(0.35, 0.45, 0.35, 1.0),
		TrackKind::Subtitle => hsla(0.10, 0.45, 0.35, 1.0),
	}
}
