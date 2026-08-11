//! Track headers: the per-track control column left of the clip area.
//!
//! One [`TrackHeader`] per track shows the track name and the toggle buttons
//! appropriate for its [`TrackKind`], and hosts the drag separator that
//! resizes the track's height.
//!
//! # Toggles per track kind
//!
//! | [`TrackKind`]             | Buttons                     |
//! |---------------------------|-----------------------------|
//! | [`TrackKind::Video`]      | lock, show (visibility)     |
//! | [`TrackKind::Audio`]      | lock, mute, solo            |
//! | [`TrackKind::Subtitle`]   | lock, show                  |
//!
//! Toggles emit [`TrackHeaderEvent`]s; like every other edit surface of this
//! module the header never mutates the model — the host applies the change
//! through its engine and the next data read reflects it.
//!
//! The element is purely visual (like [`TimelineRuler`](super::TimelineRuler)):
//! it paints the name and the toggle state glyphs, while the click handlers
//! that turn a press into a [`TrackHeaderEvent`] are attached by
//! [`TimelineView`](super::TimelineView)'s interactive wrapper.

use crate::{App, Hsla, SharedString, Window, div, hsla, prelude::*, px};

use super::data::TrackKind;

/// A user action on a track header's controls.
///
/// The host applies these to its track state (Oak: the facade's track
/// lock/mute/solo/show setters, wrapped in undo where the engine treats them
/// as undoable) and notifies the data source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackHeaderEvent {
	/// Lock toggle requested. Locked tracks reject all clip edits.
	ToggleLock,
	/// Mute toggle requested (audio tracks).
	ToggleMute,
	/// Solo toggle requested (audio tracks).
	ToggleSolo,
	/// Visibility (show) toggle requested (video/subtitle tracks).
	ToggleVisibility,
}

/// The header control for one track, rendered in the left column.
///
/// The header fills its cell in the view's row layout; the bottom
/// [`Self::SEPARATOR_HEIGHT`] pixels are the height-resize drag zone.
#[derive(IntoElement)]
pub struct TrackHeader {
	index: usize,
	name: SharedString,
	kind: TrackKind,
	locked: bool,
	muted: bool,
	solo: bool,
	visible: bool,
}

impl TrackHeader {
	/// Height of the drag separator zone at the header's bottom edge, in
	/// pixels.
	pub const SEPARATOR_HEIGHT: f32 = 4.0;

	/// Creates a header for the track at `index` from its
	/// [`TrackData`](super::TrackData) snapshot.
	pub fn new(index: usize, name: SharedString, kind: TrackKind) -> Self {
		TrackHeader {
			index,
			name,
			kind,
			locked: false,
			muted: false,
			solo: false,
			visible: true,
		}
	}

	/// Builder: current lock state (drives the lock button's active style).
	pub fn locked(mut self, locked: bool) -> Self {
		self.locked = locked;
		self
	}

	/// Builder: current mute state. Only meaningful for
	/// [`TrackKind::Audio`]; ignored otherwise.
	pub fn muted(mut self, muted: bool) -> Self {
		self.muted = muted;
		self
	}

	/// Builder: current solo state. Only meaningful for
	/// [`TrackKind::Audio`]; ignored otherwise.
	pub fn solo(mut self, solo: bool) -> Self {
		self.solo = solo;
		self
	}

	/// Builder: current visibility (show) state. Only meaningful for
	/// [`TrackKind::Video`] and [`TrackKind::Subtitle`]; ignored otherwise.
	pub fn visible(mut self, visible: bool) -> Self {
		self.visible = visible;
		self
	}

	/// Index of the track this header controls.
	pub fn track_index(&self) -> usize {
		self.index
	}

	/// The background tint for a track of `kind`.
	fn kind_background(kind: TrackKind) -> Hsla {
		match kind {
			TrackKind::Video => hsla(0.58, 0.45, 0.32, 0.18),
			TrackKind::Audio => hsla(0.35, 0.45, 0.32, 0.18),
			TrackKind::Subtitle => hsla(0.10, 0.45, 0.32, 0.18),
		}
	}

	/// The label color for a track of `kind`.
	fn kind_text(kind: TrackKind) -> Hsla {
		match kind {
			TrackKind::Video => hsla(0.58, 0.35, 0.85, 1.0),
			TrackKind::Audio => hsla(0.35, 0.35, 0.85, 1.0),
			TrackKind::Subtitle => hsla(0.10, 0.35, 0.85, 1.0),
		}
	}

	/// A small toggle glyph (one or two letters) reflecting `active`.
	fn toggle_glyph(&self, label: &str, active: bool) -> impl IntoElement {
		div()
			.px_1()
			.rounded(px(3.))
			.text_xs()
			.font_weight(if active {
				crate::FontWeight::BOLD
			} else {
				crate::FontWeight::NORMAL
			})
			.text_color(if active {
				hsla(0.63, 0.6, 0.65, 1.0)
			} else {
				hsla(0.0, 0.0, 0.5, 0.55)
			})
			.child(label.to_string())
	}

	/// The kind-appropriate toggle glyphs, left of the separator.
	fn toggle_row(&self) -> impl IntoElement {
		let lock = self.toggle_glyph("L", self.locked);
		match self.kind {
			TrackKind::Audio => div()
				.flex()
				.flex_row()
				.items_center()
				.gap(px(3.))
				.child(lock)
				.child(self.toggle_glyph("M", self.muted))
				.child(self.toggle_glyph("S", self.solo)),
			TrackKind::Video | TrackKind::Subtitle => div()
				.flex()
				.flex_row()
				.items_center()
				.gap(px(3.))
				.child(lock)
				.child(self.toggle_glyph("V", self.visible)),
		}
	}
}

impl RenderOnce for TrackHeader {
	fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
		let background = Self::kind_background(self.kind);
		let text = Self::kind_text(self.kind);
		let separator_height = px(TrackHeader::SEPARATOR_HEIGHT);

		div()
			.size_full()
			.bg(background)
			.flex()
			.flex_col()
			.child(
				div()
					.flex_1()
					.flex()
					.flex_row()
					.items_center()
					.gap(px(6.))
					.px_2()
					.child(div().text_sm().text_color(text).child(self.name.clone()))
					.child(div().flex_1())
					.child(self.toggle_row()),
			)
			.child(
				div()
					.h(separator_height)
					.w_full()
					.bg(hsla(0.0, 0.0, 0.5, 0.25)),
			)
	}
}
