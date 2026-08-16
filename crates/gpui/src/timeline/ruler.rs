//! The ruler: timecode ticks, labels, work-area band, and seek handling.
//!
//! [`TimelineRuler`] is a custom element (built on [`canvas`](crate::canvas))
//! painted across the top of [`TimelineView`](super::TimelineView). It shares
//! the clip area's frame↔pixel mapping via [`TimelineState`], so ticks and
//! clips stay aligned at every zoom level.
//!
//! # Adaptive tick spacing
//!
//! The ruler picks the smallest "nice" step whose on-screen spacing is at
//! least [`TimelineRuler::MIN_TICK_SPACING`], walking this ladder (in frames, at the
//! sequence's [`FrameRate`]):
//!
//! `1, 2, 5, 10, 30, 1 s, 2 s, 5 s, 10 s, 30 s, 1 min, 5 min, 10 min, …`
//!
//! so labels never overlap whether you're zoomed to a single frame or to a
//! two-hour sequence. Major ticks get a
//! [`format_timecode`](super::format_timecode) label; minor ticks are drawn
//! shorter and unlabeled.

use crate::{
	App, Bounds, Font, Hsla, SharedString, TextAlign, TextRun, Window, canvas, fill, hsla, point,
	prelude::*, px, size,
};

use super::{
	state::TimelineState,
	time::{Frame, FrameRange, FrameRate, TimeDisplay},
};

/// A marker to paint on the ruler, at a sequence frame with its color.
#[derive(Debug, Clone)]
pub struct RulerMarker {
	/// The marker's frame position.
	pub frame: Frame,
	/// The marker's color (the theme accent is the fallback).
	pub color: Hsla,
}

/// The sequence ruler rendered above the tracks.
///
/// Cheap to construct — all fields are plain data copied out of the view
/// each frame. The element is purely visual: it paints ticks, labels and the
/// work-area band on a [`canvas`](crate::canvas). Hit-testing and seeking
/// (mouse-down = move the playhead, dragging the work-area band edges) are
/// wired up by [`TimelineView`](super::TimelineView)'s interactive wrapper,
/// because elements cannot emit events.
#[derive(IntoElement)]
pub struct TimelineRuler {
	state: TimelineState,
	frame_rate: FrameRate,
	sequence_length: Frame,
	display: TimeDisplay,
	/// Markers to paint (from the data source), in ascending frame order.
	markers: Vec<RulerMarker>,
}

impl TimelineRuler {
	/// Minimum on-screen distance between two labeled ticks, in pixels. The
	/// adaptive step ladder never picks a step smaller than this.
	pub const MIN_TICK_SPACING: f32 = 80.0;

	/// Creates a ruler element snapshotting the given view state.
	///
	/// * `state` — supplies zoom and horizontal scroll; the ruler shares the
	///   clip area's mapping exactly.
	/// * `frame_rate` / `sequence_length` — from the
	///   [`TimelineDataSource`](super::TimelineDataSource).
	pub fn new(state: TimelineState, frame_rate: FrameRate, sequence_length: Frame) -> Self {
		TimelineRuler {
			state,
			frame_rate,
			sequence_length,
			display: TimeDisplay::default(),
			markers: Vec::new(),
		}
	}

	/// Builder: how to label major ticks. Defaults to
	/// [`TimeDisplay::Timecode`].
	pub fn time_display(mut self, display: TimeDisplay) -> Self {
		self.display = display;
		self
	}

	/// Builder: the markers to paint (sequence markers from the data
	/// source). Drawn as small diamonds below the tick baseline.
	pub fn markers(mut self, markers: Vec<RulerMarker>) -> Self {
		self.markers = markers;
		self
	}

	/// The tick step (in frames) the ruler would choose at the given zoom.
	///
	/// Exposed for tests and for snapping the playhead-drag indicator to the
	/// visible grid. Must return a value from the "nice step" ladder
	/// described in the module docs such that
	/// `step * zoom >= Self::MIN_TICK_SPACING` for all but the coarsest
	/// step.
	pub fn tick_step(&self, zoom: f32) -> Frame {
		// Nominal (integer) frames per second, matching the non-drop-frame
		// convention used by `format_timecode` (NTSC 29.97 labels in 30 fps).
		let fps = self.frame_rate.as_f64().round() as i64;
		// The "nice step" ladder, finest to coarsest, in frames:
		// 1, 2, 5, 10, 30 (frames), 1/2/5/10/30 seconds, 1/5/10/30 minutes,
		// 1/2 hours.
		let ladder = [
			1,
			2,
			5,
			10,
			30,
			fps,
			2 * fps,
			5 * fps,
			10 * fps,
			30 * fps,
			60 * fps,
			300 * fps,
			600 * fps,
			1800 * fps,
			3600 * fps,
			7200 * fps,
		];
		for step in ladder {
			if step as f32 * zoom >= Self::MIN_TICK_SPACING {
				return Frame(step);
			}
		}
		// Coarsest step; the spacing contract allows the last ladder entry to
		// fall short of `MIN_TICK_SPACING`.
		Frame(7200 * fps)
	}

	/// The work-area band to paint, if any.
	pub fn work_area(&self) -> Option<FrameRange> {
		self.state.work_area
	}

	/// Label text for a major tick at `frame`, per
	/// [`Self::time_display`].
	pub fn tick_label(&self, frame: Frame) -> SharedString {
		super::time::format_timecode(frame, self.frame_rate, self.display).into()
	}
}

/// A single ruler tick computed during canvas prepaint.
struct RulerTick {
	/// Local x within the ruler (relative to its left edge, which aligns
	/// with the clip area's left edge).
	x: f32,
	/// Whether this is a major (tall, labeled) tick.
	major: bool,
	/// The label for major ticks.
	label: Option<SharedString>,
}

/// Everything the canvas paint closure needs, computed in prepaint.
struct RulerContent {
	ticks: Vec<RulerTick>,
	/// Local x-extents `(left, right)` of the work-area band, if a work
	/// area is set.
	work_area: Option<(f32, f32)>,
	/// Marker positions `(x, color)` for the visible markers.
	markers: Vec<(f32, Hsla)>,
}

impl RenderOnce for TimelineRuler {
	fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
		canvas(
			move |bounds, _window, _cx| {
				// All state is captured by moving `self` into this prepaint
				// closure; the paint closure only needs the precomputed
				// `RulerContent`, so it borrows nothing from `self`.
				let state = &self.state;
				let work_area = self.work_area();
				let step = self.tick_step(state.zoom);
				let sequence_length = self.sequence_length;

				// First and last on-screen frames in ruler-local coordinates,
				// which match the clip area's, so `TimelineState`'s
				// frame↔pixel mapping applies directly. The last frame is
				// clamped to the sequence so the ruler doesn't draw an
				// endless row of ticks if the view is scrolled far right.
				let first = state.frame_at_point(px(0.0));
				let last = Frame(
					state
						.frame_at_point(bounds.size.width)
						.0
						.min(sequence_length.0),
				);
				// Align to multiples of `step` so ticks stay put relative to
				// the clip grid while scrolling.
				let start = Frame(first.0.div_euclid(step.0) * step.0);

				let mut ticks = Vec::new();
				let mut frame = start;
				while frame <= last {
					ticks.push(RulerTick {
						x: state.point_at_frame(frame).0,
						major: true,
						label: Some(self.tick_label(frame)),
					});
					frame = frame + step;
				}

				// Minor ticks at the midpoint between majors, only when they
				// keep enough pixel separation to be legible.
				if step.0 >= 2 && (step.0 as f32 / 2.0) * state.zoom >= 4.0 {
					let mut frame = Frame(start.0 + step.0 / 2);
					while frame <= last {
						ticks.push(RulerTick {
							x: state.point_at_frame(frame).0,
							major: false,
							label: None,
						});
						frame = frame + step;
					}
				}

				// Markers within the visible span, in ascending frame order
				// (the data source provides them sorted).
				let markers = self
					.markers
					.iter()
					.filter(|m| m.frame >= first && m.frame <= last)
					.map(|m| (state.point_at_frame(m.frame).0, m.color))
					.collect();

				RulerContent {
					ticks,
					work_area: work_area.map(|range| {
						(
							state.point_at_frame(range.start).0,
							state.point_at_frame(range.end).0,
						)
					}),
					markers,
				}
			},
			move |bounds, content, window, cx| {
				let baseline_color = hsla(0.0, 0.0, 0.5, 0.5);
				let major_color = hsla(0.0, 0.0, 0.6, 0.9);
				let minor_color = hsla(0.0, 0.0, 0.6, 0.45);
				let text_color = hsla(0.0, 0.0, 0.5, 1.0);
				let band_color = hsla(0.63, 0.55, 0.55, 0.10);
				let bottom = bounds.bottom();

				// Work-area band under the ticks, with edge lines.
				if let Some((left, right)) = content.work_area {
					let width = px((right - left).max(0.0));
					let left = bounds.left() + px(left);
					let band = Bounds {
						origin: point(left, bounds.top()),
						size: size(width, bounds.size.height),
					};
					window.paint_quad(fill(band, band_color));
					for edge in [left.0, left.0 + width.0] {
						window.paint_quad(fill(
							Bounds {
								origin: point(px(edge), bounds.top()),
								size: size(px(1.0), bounds.size.height),
							},
							band_color,
						));
					}
				}

				// Markers as small diamonds hanging below the tick baseline.
				let marker_half: f32 = 3.0;
				for (x, color) in &content.markers {
					let cx = bounds.left() + px(*x);
					let cy = bounds.bottom() - px(4.0);
					let mut path = crate::path_builder::PathBuilder::fill();
					path.add_polygon(
						&[
							point(cx, cy - px(marker_half)),
							point(cx + px(marker_half), cy),
							point(cx, cy + px(marker_half)),
							point(cx - px(marker_half), cy),
						],
						true,
					);
					if let Ok(path) = path.build() {
						window.paint_path(path, *color);
					}
				}

				// Baseline along the bottom of the ruler.
				window.paint_quad(fill(
					Bounds {
						origin: point(bounds.left(), bottom - px(1.0)),
						size: size(bounds.size.width, px(1.0)),
					},
					baseline_color,
				));

				for tick in content.ticks {
					let x = bounds.left() + px(tick.x);
					let height = if tick.major { 16.0 } else { 8.0 };
					window.paint_quad(fill(
						Bounds {
							origin: point(x, bottom - px(height)),
							size: size(px(1.0), px(height)),
						},
						if tick.major { major_color } else { minor_color },
					));
					if let Some(label) = tick.label {
						let len = label.len();
						let line = window.text_system().shape_line(
							label,
							px(11.0),
							&[TextRun {
								len,
								font: Font::default(),
								color: text_color,
								background_color: None,
								underline: None,
								strikethrough: None,
								letter_spacing: None,
							}],
							None,
						);
						let _ = line.paint(
							point(px(x.0 + 4.0), bottom - px(23.0)),
							px(12.0),
							TextAlign::Left,
							None,
							window,
							cx,
						);
					}
				}
			},
		)
		.size_full()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn tick_step_returns_nice_ladder_step() {
		let ruler = TimelineRuler::new(
			TimelineState::new(),
			FrameRate::new(30, 1),
			Frame(30 * 60 * 60),
		);
		// At 100 px/frame a single frame spans 100 px: the finest step wins.
		assert_eq!(ruler.tick_step(100.0), Frame(1));
		// At 3 px/frame a 1-second step spans 90 px: the smallest ladder
		// entry that clears MIN_TICK_SPACING is one second (30 frames).
		assert_eq!(ruler.tick_step(3.0), Frame(30));
		// At 1/3000 px/frame a 2-hour step spans 64.8 px, under the minimum;
		// the ladder's last entry is the fallback.
		assert_eq!(ruler.tick_step(0.0003), Frame(7200 * 30));
	}

	#[test]
	fn tick_step_uses_nominal_fps_for_fractional_rates() {
		// NTSC 29.97 labels ticks in nominal 30 fps, matching
		// `format_timecode`'s non-drop-frame convention.
		let ruler = TimelineRuler::new(
			TimelineState::new(),
			FrameRate::NTSC_2997,
			Frame(30 * 60 * 60),
		);
		assert_eq!(ruler.tick_step(3.0), Frame(30));
	}

	#[test]
	fn tick_step_keeps_labels_apart() {
		let ruler = TimelineRuler::new(
			TimelineState::new(),
			FrameRate::new(25, 1),
			Frame(25 * 60 * 60),
		);
		for zoom in [0.001, 0.01, 0.1, 0.5, 1.0, 3.0, 10.0, 100.0, 1000.0] {
			let step = ruler.tick_step(zoom);
			// The coarsest step (2 hours at 25 fps) is the documented
			// exception to the spacing contract.
			if step.0 != 7200 * 25 {
				assert!(
					step.0 as f32 * zoom >= TimelineRuler::MIN_TICK_SPACING,
					"zoom {zoom}: step {step:?} under-spaces"
				);
			}
		}
	}
}
