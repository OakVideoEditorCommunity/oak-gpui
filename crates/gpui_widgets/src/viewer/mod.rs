//! The viewer widget: a fullscreen preview with a transport bar.
//!
//! The picture comes from a [`SurfaceSource`] (in Oak, the W3 bridge's
//! IOSurface-backed pixel buffer); the playhead position is polled from the
//! host's [`PlaybackClock`] on a ~60 Hz timer, and every transport action is
//! emitted as a [`ViewerEvent`] request (the engine applies it and the clock
//! reflects it). Timecode formatting reuses `gpui::timeline::time`.

pub mod clock;
pub mod transport;

pub use clock::*;
pub use transport::*;

use gpui::timeline::{FrameRate, TimeDisplay, format_timecode};
use gpui::{
	AnyElement, App, AsyncWindowContext, Bounds, ClickEvent, Context, Entity, EventEmitter,
	FocusHandle, Focusable, Keystroke, KeyDownEvent, KeyUpEvent, MouseButton, MouseMoveEvent,
	ObjectFit, Point, Pixels, Render, RenderImage, SharedString, SurfaceSource, Window,
	canvas, colors::DefaultColors, div, img, prelude::*, px, surface,
};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use crate::{icons, tooltip::tooltip_view};

/// A request emitted by the viewer.
#[derive(Debug, Clone, PartialEq)]
pub enum ViewerEvent {
	/// Start playback.
	PlayRequested {
		/// The viewer's stable id.
		control: usize,
	},
	/// Pause playback.
	PauseRequested {
		/// The viewer's stable id.
		control: usize,
	},
	/// Step the playhead by `delta` frames.
	StepRequested {
		/// The viewer's stable id.
		control: usize,
		/// Frames to step (negative steps backward).
		delta: i64,
	},
	/// Set the loop-in point at the playhead.
	InPointRequested {
		/// The viewer's stable id.
		control: usize,
	},
	/// Set the loop-out point at the playhead.
	OutPointRequested {
		/// The viewer's stable id.
		control: usize,
	},
	/// Clear the loop range.
	ClearRangeRequested {
		/// The viewer's stable id.
		control: usize,
	},
	/// Toggle the safe-frame overlay.
	ToggleSafeFramesRequested {
		/// The viewer's stable id.
		control: usize,
	},
	/// Toggle the zoom (contain vs cover).
	ToggleZoomRequested {
		/// The viewer's stable id.
		control: usize,
	},
	/// A pointer (pen) event inside the picture area, emitted so the host
	/// can forward it to an OFX interact. `position` is local to the
	/// picture area (top-left origin, logical pixels).
	InteractPointer {
		/// Whether this is a move, a press or a release.
		kind: InteractPointerKind,
		/// Position in the picture area's local pixels (top-left origin).
		position: Point<f32>,
		/// The button involved in a press/release, if any.
		button: Option<MouseButton>,
		/// Whether a pointer button is held at this event (the pen-down
		/// state for a move).
		pressed: bool,
	},
	/// A key press/release while the picture area has keyboard focus,
	/// emitted for OFX interact forwarding. Keys consumed by a global
	/// keybinding never reach this event (gpui dispatches bindings before
	/// the focus-path key handlers).
	InteractKey {
		/// true = press, false = release.
		down: bool,
		/// The keystroke (modifiers + key name).
		keystroke: Keystroke,
	},
}

/// The kind of a pointer event inside the picture area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractPointerKind {
	/// A mouse move (possibly with a button held).
	Move,
	/// A mouse button press.
	Down,
	/// A mouse button release.
	Up,
}

/// The zoom state of the viewer picture.
///
/// [`ViewerZoom::Fit`] fits the whole frame inside the picture area (like the
/// old contain mode); [`ViewerZoom::Level`] renders the frame at a fixed
/// multiple of its native resolution, clipped to the picture area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerZoom {
	/// Fit the entire frame (contain).
	Fit,
	/// A fixed multiple of the native resolution (index into
	/// [`VIEWER_ZOOM_LEVELS`]).
	Level(usize),
}

impl ViewerZoom {
	/// The next zoom in the cycle used by the toolbar zoom button: Fit → 100%
	/// → 200% → Fit; any other level steps up by one.
	pub fn next(self) -> Self {
		match self {
			ViewerZoom::Fit => ViewerZoom::Level(4),
			ViewerZoom::Level(i) if i + 1 >= VIEWER_ZOOM_LEVELS.len() => ViewerZoom::Fit,
			ViewerZoom::Level(i) if i == 3 => ViewerZoom::Level(7),
			ViewerZoom::Level(i) => ViewerZoom::Level(i + 1),
		}
	}
}

/// The safe-frame margin overlay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SafeMargins {
	/// No overlay.
	Off,
	/// The standard 90% × 80% action-safe margins.
	On,
	/// Custom horizontal/vertical fractions of the frame (e.g. 0.9 × 0.8).
	Custom(f32, f32),
}

impl SafeMargins {
	/// The next state in the toolbar cycle: Off → On → Off; custom behaves
	/// like On (cycles back to Off).
	pub fn next(self) -> Self {
		match self {
			SafeMargins::Off => SafeMargins::On,
			SafeMargins::On | SafeMargins::Custom(_, _) => SafeMargins::Off,
		}
	}

	/// The frame fractions the overlay covers, or `None` when disabled.
	pub fn ratio(self) -> Option<(f32, f32)> {
		match self {
			SafeMargins::Off => None,
			SafeMargins::On => Some((0.9, 0.8)),
			SafeMargins::Custom(w, h) => Some((w, h)),
		}
	}
}

/// How the audio waveform overlay behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveformMode {
	/// Show the waveform only while playing.
	Automatic = 0,
	/// Show the waveform always (never hide it).
	Only = 1,
	/// Show both the waveform and the picture.
	Both = 2,
}

impl WaveformMode {
	/// The persisted config value (`0`/`1`/`2`).
	pub fn config_value(self) -> i32 {
		self as i32
	}

	/// Reconstruct from a persisted config value, clamping out-of-range
	/// values back to [`WaveformMode::Automatic`].
	pub fn from_config_value(value: i32) -> Self {
		match value.clamp(0, 2) {
			1 => WaveformMode::Only,
			2 => WaveformMode::Both,
			_ => WaveformMode::Automatic,
		}
	}
}

/// The zoom levels offered by the context menu, as multiples of the native
/// resolution. Index 4 is 100% and index 7 is 200% — those are the stops the
/// toolbar zoom button cycles through.
pub const VIEWER_ZOOM_LEVELS: [f32; 10] = [
	0.10, 0.25, 0.50, 0.75, 1.00, 1.25, 1.50, 2.00, 4.00, 8.00,
];

/// A smoothed per-second frame-rate meter for the `Show FPS` overlay.
#[derive(Debug, Clone, Copy)]
pub struct FpsCounter {
	last: Option<std::time::Instant>,
	smoothed: f32,
	samples: u32,
}

impl FpsCounter {
	/// A fresh counter with no samples.
	pub fn new() -> Self {
		Self {
			last: None,
			smoothed: 0.0,
			samples: 0,
		}
	}

	/// Record a new frame at `now` and update the smoothed rate.
	pub fn record(&mut self, now: std::time::Instant) {
		if let Some(last) = self.last {
			let dt = now.duration_since(last).as_secs_f32();
			if dt > 0.0 {
				let instant = 1.0 / dt;
				self.smoothed = if self.samples == 0 {
					instant
				} else {
					self.smoothed * 0.9 + instant * 0.1
				};
				self.samples += 1;
			}
		}
		self.last = Some(now);
	}

	/// The smoothed rate, or `0.0` before the second frame.
	pub fn value(&self) -> f32 {
		self.smoothed
	}
}

impl Default for FpsCounter {
	fn default() -> Self {
		Self::new()
	}
}

/// The picture source of a [`ViewerWidget`].
///
/// On macOS the fast path is a CoreVideo [`SurfaceSource`]; on platforms
/// without CVPixelBuffer (or when the engine only produces CPU frames) use
/// [`ViewerFrameSource::CpuFrame`].
#[derive(Clone)]
pub enum ViewerFrameSource {
	/// A platform surface: a CoreVideo pixel buffer on macOS, or a GPU
	/// texture handle on Linux/FreeBSD.
	Surface(SurfaceSource),
	/// A CPU-side frame as raw bytes in a [`RenderImage`] (BGRA8, row-major,
	/// top-to-bottom), uploaded through gpui's sprite atlas on every
	/// platform — the path to use when no platform surface is available.
	CpuFrame(Arc<RenderImage>),
}

/// The viewer widget.
pub struct ViewerWidget<C: PlaybackClock> {
	control: usize,
	clock: Entity<C>,
	frame_rate: FrameRate,
	transport: TransportState,
	frame_source: Option<ViewerFrameSource>,
	focus_handle: FocusHandle,
	safe_margins: SafeMargins,
	zoom: ViewerZoom,
	show_fps: bool,
	fps: FpsCounter,
	/// The picture area's bounds in window pixels, captured each render
	/// (the interact pointer forwarding maps window events to the picture's
	/// local pixels from these).
	picture_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl<C: PlaybackClock> ViewerWidget<C> {
	/// Create a viewer driven by `clock`.
	pub fn new(
		control: usize,
		clock: Entity<C>,
		window: &mut Window,
		cx: &mut Context<Self>,
	) -> Self {
		let frame_rate = clock.read(cx).frame_rate();

		// Poll the engine clock on a timer and reflect it locally.
		let this = cx.weak_entity();
		window
			.spawn(cx, async move |cx: &mut AsyncWindowContext| {
				loop {
					cx.background_executor()
						.timer(std::time::Duration::from_millis(16))
						.await;
					let _ = cx.update(|_window, app| {
						if let Some(this) = this.upgrade() {
							this.update(app, |this, cx| this.poll_clock(cx));
						}
					});
				}
			})
			.detach();

		Self {
			control,
			clock,
			frame_rate,
			transport: TransportState::new(),
			frame_source: None,
			focus_handle: cx.focus_handle(),
			safe_margins: SafeMargins::Off,
			zoom: ViewerZoom::Fit,
			show_fps: false,
			fps: FpsCounter::new(),
			picture_bounds: Rc::new(Cell::new(None)),
		}
	}

	/// The current transport state.
	pub fn transport(&self) -> TransportState {
		self.transport
	}

	/// Set the picture source (the bridge's pixel buffer) and repaint.
	pub fn set_frame_source(&mut self, source: Option<SurfaceSource>, cx: &mut Context<Self>) {
		self.frame_source = source.map(ViewerFrameSource::Surface);
		cx.notify();
	}

	/// Set the picture source to a CPU-side frame and repaint.
	///
	/// This is the path for non-macOS platforms and engines that decode to
	/// raw pixels instead of platform surfaces: hand in a
	/// [`RenderImage`](gpui::RenderImage) whose bytes are BGRA8 (the same
	/// format gpui's `img` element uses) and the viewer uploads it through
	/// the sprite atlas. `None` clears the picture (showing the placeholder).
	pub fn set_cpu_frame(&mut self, frame: Option<Arc<RenderImage>>, cx: &mut Context<Self>) {
		self.frame_source = frame.map(ViewerFrameSource::CpuFrame);
		cx.notify();
	}

	/// The picture area's current bounds in window pixels, or `None` before
	/// the picture was first painted. The host uses these to convert the
	/// [`ViewerEvent::InteractPointer`] positions (picture-local) into the
	/// interact's viewport coordinates.
	pub fn picture_bounds(&self) -> Option<Bounds<Pixels>> {
		self.picture_bounds.get()
	}

	/// The current safe-margin overlay state.
	pub fn safe_margins(&self) -> SafeMargins {
		self.safe_margins
	}

	/// Set the safe-margin overlay state and repaint.
	pub fn set_safe_margins(&mut self, margins: SafeMargins, cx: &mut Context<Self>) {
		self.safe_margins = margins;
		cx.notify();
	}

	/// The current zoom state.
	pub fn zoom(&self) -> ViewerZoom {
		self.zoom
	}

	/// Set the zoom state and repaint.
	pub fn set_zoom(&mut self, zoom: ViewerZoom, cx: &mut Context<Self>) {
		self.zoom = zoom;
		cx.notify();
	}

	/// Whether the frame-rate overlay is shown.
	pub fn show_fps(&self) -> bool {
		self.show_fps
	}

	/// Toggle the frame-rate overlay and repaint.
	pub fn set_show_fps(&mut self, show: bool, cx: &mut Context<Self>) {
		self.show_fps = show;
		cx.notify();
	}

	/// The frame's native size in pixels when the source carries one (CPU
	/// frames), `None` for platform surfaces. Used to size the safe-frame
	/// overlay relative to the actual picture.
	fn frame_pixel_size(&self) -> Option<(f32, f32)> {
		match &self.frame_source {
			Some(ViewerFrameSource::CpuFrame(image)) => {
				let size = image.size(0);
				Some((size.width.0 as f32, size.height.0 as f32))
			}
			_ => None,
		}
	}

	fn emit_interact_pointer(
		&mut self,
		kind: InteractPointerKind,
		position: Point<Pixels>,
		button: Option<MouseButton>,
		pressed: bool,
		cx: &mut Context<Self>,
	) {
		cx.emit(ViewerEvent::InteractPointer {
			kind,
			position: Point::new(f32::from(position.x), f32::from(position.y)),
			button,
			pressed,
		});
	}

	fn poll_clock(&mut self, cx: &mut Context<Self>) {
		let clock = self.clock.read(cx);
		let frame = clock.current_frame();
		let playing = clock.is_playing();
		if frame != self.transport.frame || playing != self.transport.playing {
			self.fps.record(std::time::Instant::now());
			self.transport.frame = frame;
			self.transport.playing = playing;
			cx.notify();
		}
	}

	fn emit(&mut self, event: ViewerEvent, cx: &mut Context<Self>) {
		cx.emit(event);
		cx.notify();
	}
}

impl<C: PlaybackClock> EventEmitter<ViewerEvent> for ViewerWidget<C> {}

impl<C: PlaybackClock> Focusable for ViewerWidget<C> {
	fn focus_handle(&self, _cx: &App) -> FocusHandle {
		self.focus_handle.clone()
	}
}

impl<C: PlaybackClock> Render for ViewerWidget<C> {
	fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
		let colors = cx.default_colors().clone();
		let timecode =
			format_timecode(self.transport.frame, self.frame_rate, TimeDisplay::Timecode);

		// The picture area: surface (or placeholder), safe frames and zoom.
		let mut picture = div()
			.id("gpui-widgets-viewer-picture")
			.flex_1()
			.min_w_0()
			.min_h_0()
			.overflow_hidden()
			.relative()
			.bg(gpui::Hsla {
				h: 0.0,
				s: 0.0,
				l: 0.0,
				a: 1.0,
			})
			// The picture is focusable so keyboard events reach the OFX
			// interact forwarding while the user is interacting with the
			// picture (the panel decides what an active interact consumes).
			.focusable()
			.on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _window, cx| {
				if let Some(bounds) = this.picture_bounds.get() {
					this.emit_interact_pointer(
						InteractPointerKind::Move,
						event.position - bounds.origin,
						None,
						event.pressed_button.is_some(),
						cx,
					);
				}
			}))
			.on_mouse_down(
				MouseButton::Left,
				cx.listener(|this, event: &gpui::MouseDownEvent, _window, cx| {
					if let Some(bounds) = this.picture_bounds.get() {
						this.emit_interact_pointer(
							InteractPointerKind::Down,
							event.position - bounds.origin,
							Some(event.button),
							true,
							cx,
						);
					}
				}),
			)
			.on_mouse_up(
				MouseButton::Left,
				cx.listener(|this, event: &gpui::MouseUpEvent, _window, cx| {
					if let Some(bounds) = this.picture_bounds.get() {
						this.emit_interact_pointer(
							InteractPointerKind::Up,
							event.position - bounds.origin,
							Some(event.button),
							false,
							cx,
						);
					}
				}),
			)
			.on_key_down(cx.listener(|_this, event: &KeyDownEvent, _window, cx| {
				cx.emit(ViewerEvent::InteractKey {
					down: true,
					keystroke: event.keystroke.clone(),
				});
			}))
			.on_key_up(cx.listener(|_this, event: &KeyUpEvent, _window, cx| {
				cx.emit(ViewerEvent::InteractKey {
					down: false,
					keystroke: event.keystroke.clone(),
				});
			}));

		if let Some(source) = &self.frame_source {
			let picture_element: AnyElement = match self.zoom {
				// Fit renders the whole frame, contained inside the area.
				ViewerZoom::Fit => match source {
					ViewerFrameSource::Surface(surface_source) => surface(surface_source.clone())
						.size_full()
						.object_fit(ObjectFit::Contain)
						.into_any(),
					ViewerFrameSource::CpuFrame(image) => img(image.clone())
						.size_full()
						.min_w_0()
						.min_h_0()
						.object_fit(ObjectFit::Contain)
						.into_any(),
				},
				// A zoom level renders at a fixed multiple of the native
				// resolution. Only a CPU frame carries its pixel size; a
				// platform surface has no resolution here, so it falls back
				// to fit.
				ViewerZoom::Level(i) => match source {
					ViewerFrameSource::Surface(surface_source) => surface(surface_source.clone())
						.size_full()
						.object_fit(ObjectFit::Contain)
						.into_any(),
					ViewerFrameSource::CpuFrame(image) => {
						let scale = VIEWER_ZOOM_LEVELS[i];
						let size = image.size(0);
						div()
							.absolute()
							.left_0()
							.right_0()
							.top_0()
							.bottom_0()
							.flex()
							.items_center()
							.justify_center()
							.child(
								img(image.clone())
									.w(px(size.width.0 as f32 * scale))
									.h(px(size.height.0 as f32 * scale))
									.object_fit(ObjectFit::Fill),
							)
							.into_any()
					}
				},
			};
			picture = picture.child(picture_element);
		} else {
			picture = picture.child(
				div()
					.size_full()
					.flex()
					.items_center()
					.justify_center()
					.text_color(colors.disabled)
					.child(crate::i18n::tr("viewer.no_frame_source", "No frame source")),
			);
		}

		if let (Some((frame_w, frame_h)), Some((margin_w, margin_h))) =
			(self.frame_pixel_size(), self.safe_margins.ratio())
		{
			picture = picture.child(
				div()
					.absolute()
					.left_0()
					.right_0()
					.top_0()
					.bottom_0()
					.flex()
					.items_center()
					.justify_center()
					.child(
						div()
							.w(px(frame_w * margin_w))
							.h(px(frame_h * margin_h))
							.border_1()
							.border_color(colors.selected),
					),
			);
		}

		if self.show_fps {
			picture = picture.child(
				div()
					.absolute()
					.right_1()
					.top_1()
					.px_1()
					.py_0p5()
					.rounded_md()
					.bg(colors.container)
					.text_xs()
					.text_color(colors.text)
					.child(format!("{:.1} fps", self.fps.value())),
			);
		}

		// Capture the picture area's bounds each frame so the pointer
		// handlers above can map window coordinates to picture-local
		// pixels. The canvas paints nothing; it only records bounds.
		let picture_bounds = self.picture_bounds.clone();
		picture = picture.child(
			canvas(
				move |bounds, _window, _cx| {
					picture_bounds.set(Some(bounds));
				},
				|_bounds, (), _window, _cx| {},
			)
			.absolute()
			.left_0()
			.right_0()
			.top_0()
			.bottom_0(),
		);

		// Transport bar. The transport controls are icon buttons (16px icon
		// on a 24px hit target, localized tooltips); without a registered
		// icon resolver the buttons fall back to the glyph labels below.
		let playing = self.transport.playing;
		let play_label = if playing { "⏸" } else { "▶" };
		let in_icon = icons::path("prev", cx);
		let step_back_icon = icons::path("rew", cx);
		let play_icon = icons::path(if playing { "pause" } else { "play" }, cx);
		let step_forward_icon = icons::path("ff", cx);
		let out_icon = icons::path("next", cx);
		let transport_bar = div()
			.flex()
			.items_center()
			.gap_2()
			.px_2()
			.py_0p5()
			.bg(colors.container)
			.border_t_1()
			.border_color(colors.border)
			.child(transport_button(
				"gpui-widgets-viewer-in",
				in_icon,
				"⏮",
				crate::i18n::tr("viewer.in_point", "入点"),
				false,
				&colors,
				cx.listener(|this, _event: &ClickEvent, _window, cx| {
					this.emit(
						ViewerEvent::InPointRequested {
							control: this.control,
						},
						cx,
					);
				}),
			))
			.child(transport_button(
				"gpui-widgets-viewer-step-back",
				step_back_icon,
				"⏪",
				crate::i18n::tr("viewer.step_back", "上一帧"),
				false,
				&colors,
				cx.listener(|this, _event: &ClickEvent, _window, cx| {
					this.emit(
						ViewerEvent::StepRequested {
							control: this.control,
							delta: -1,
						},
						cx,
					);
				}),
			))
			.child(transport_button(
				"gpui-widgets-viewer-play",
				play_icon,
				play_label,
				if playing {
					crate::i18n::tr("viewer.pause", "暂停")
				} else {
					crate::i18n::tr("viewer.play", "播放")
				},
				true,
				&colors,
				cx.listener(|this, _event: &ClickEvent, _window, cx| {
					let event = if this.transport.playing {
						ViewerEvent::PauseRequested {
							control: this.control,
						}
					} else {
						ViewerEvent::PlayRequested {
							control: this.control,
						}
					};
					this.emit(event, cx);
				}),
			))
			.child(transport_button(
				"gpui-widgets-viewer-step-forward",
				step_forward_icon,
				"⏩",
				crate::i18n::tr("viewer.step_forward", "下一帧"),
				false,
				&colors,
				cx.listener(|this, _event: &ClickEvent, _window, cx| {
					this.emit(
						ViewerEvent::StepRequested {
							control: this.control,
							delta: 1,
						},
						cx,
					);
				}),
			))
			.child(transport_button(
				"gpui-widgets-viewer-out",
				out_icon,
				"⏭",
				crate::i18n::tr("viewer.out_point", "出点"),
				false,
				&colors,
				cx.listener(|this, _event: &ClickEvent, _window, cx| {
					this.emit(
						ViewerEvent::OutPointRequested {
							control: this.control,
						},
						cx,
					);
				}),
			))
			.child(transport_button(
				"gpui-widgets-viewer-clear-range",
				None,
				x_glyph(colors.text),
				crate::i18n::tr("viewer.clear_range", "清除入出点"),
				false,
				&colors,
				cx.listener(|this, _event: &ClickEvent, _window, cx| {
					this.emit(
						ViewerEvent::ClearRangeRequested {
							control: this.control,
						},
						cx,
					);
				}),
			))
			// The current timecode reads in the design's bright green.
			.child(
				div()
					.px_2()
					.text_xs()
					.text_color(gpui::hsla(0.33, 0.75, 0.62, 1.0))
					.child(timecode),
			)
			.child(div().flex_1())
			.child(button(
				"gpui-widgets-viewer-safe",
				crate::i18n::tr("viewer.safe_frames", "安全框"),
				&colors,
				cx.listener(|this, _event: &ClickEvent, _window, cx| {
					this.safe_margins = this.safe_margins.next();
					this.emit(
						ViewerEvent::ToggleSafeFramesRequested {
							control: this.control,
						},
						cx,
					);
				}),
			))
			.child(button(
				"gpui-widgets-viewer-zoom",
				crate::i18n::tr("viewer.zoom", "缩放"),
				&colors,
				cx.listener(|this, _event: &ClickEvent, _window, cx| {
					this.zoom = this.zoom.next();
					this.emit(
						ViewerEvent::ToggleZoomRequested {
							control: this.control,
						},
						cx,
					);
				}),
			));

		div()
			.size_full()
			.flex()
			.flex_col()
			.min_w_0()
			.overflow_hidden()
			.child(picture)
			.child(transport_bar)
	}
}

/// A transport icon button: a 16px icon on a 24px hit target with a
/// localized tooltip. Falls back to the `fallback` glyph when `icon` is
/// `None` (no resolver registered, or no file for the name). `primary` marks
/// the design's accent-filled play/pause button; the rest are flat buttons
/// that surface on hover.
fn transport_button(
	id: &'static str,
	icon: Option<std::path::PathBuf>,
	fallback: impl IntoElement,
	tooltip: SharedString,
	primary: bool,
	colors: &gpui::colors::Colors,
	on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
	let background = colors.selected;
	let hover = colors.container;
	let mut el = div()
		.id(id)
		.debug_selector(move || id.into())
		.w(px(22.0))
		.h(px(22.0))
		.flex()
		.items_center()
		.justify_center()
		.rounded_md()
		.cursor_pointer()
		.text_color(if primary {
			colors.selected_text
		} else {
			colors.text
		})
		.when(primary, |style| style.bg(background))
		.hover(move |style| {
			if primary {
				style.bg(background)
			} else {
				style.bg(hover)
			}
		})
		.tooltip(move |window, cx| tooltip_view(tooltip.clone(), window, cx))
		.on_click(on_click);
	if let Some(path) = icon {
		el = el.child(img(path).w(px(16.0)).h(px(16.0)));
	} else {
		el = el.child(fallback);
	}
	el
}

/// A small painted ✕ (clear-range / close), drawn with a canvas so it stays
/// crisp and theme-colored instead of relying on a font glyph that may
/// rasterize faintly or not at all.
fn x_glyph(color: gpui::Rgba) -> impl IntoElement {
	use gpui::{Bounds, PathBuilder, Pixels, canvas, point, px};

	canvas(
		move |_bounds, _window, _cx| (),
		move |bounds: Bounds<Pixels>, (), window, cx| {
			let _ = cx;
			// Two diagonal strokes across the 16px box, with a small inset so
			// the mark reads as a clean X.
			let inset = px(4.0);
			for stroke in [true, false] {
				let mut path = PathBuilder::stroke(px(1.5));
				let (x0, x1) = if stroke {
					(bounds.left() + inset, bounds.right() - inset)
				} else {
					(bounds.right() - inset, bounds.left() + inset)
				};
				path.move_to(point(x0, bounds.top() + inset));
				path.line_to(point(x1, bounds.bottom() - inset));
				if let Ok(path) = path.build() {
					window.paint_path(path, color);
				}
			}
		},
	)
}

/// A small labeled button, styled as the design's bordered chip (the "适合 /
/// 安全框" controls at the transport bar's right end).
fn button(
	id: &'static str,
	label: impl IntoElement,
	colors: &gpui::colors::Colors,
	on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
	let border = colors.border;
	let hover = colors.separator;
	div()
		.id(id)
		.debug_selector(move || id.into())
		.px_2()
		.py_0p5()
		.rounded_md()
		.border_1()
		.border_color(border)
		.text_color(colors.text)
		.cursor_pointer()
		.hover(move |style| style.bg(hover))
		.on_click(on_click)
		.child(label)
}

#[cfg(test)]
mod tests {
	use super::*;
	use gpui::timeline::{Frame, FrameRate};
	use gpui::{Modifiers, TestAppContext, VisualTestContext, px, size};

	struct MockClock {
		frame: Frame,
		playing: bool,
	}
	impl PlaybackClock for MockClock {
		fn current_frame(&self) -> Frame {
			self.frame
		}
		fn is_playing(&self) -> bool {
			self.playing
		}
		fn frame_rate(&self) -> FrameRate {
			FrameRate::new(30, 1)
		}
	}

	struct Host {
		viewer: Entity<ViewerWidget<MockClock>>,
		events: Vec<ViewerEvent>,
	}
	impl Render for Host {
		fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
			div().size_full().child(self.viewer.clone())
		}
	}

	#[gpui::test]
	async fn play_button_emits_play_request(cx: &mut TestAppContext) {
		let (cx, host) = make_host(cx);
		// The play button sits on the left of the transport bar at the bottom.
		let play = cx
			.debug_bounds("gpui-widgets-viewer-play")
			.expect("play button rendered");
		cx.simulate_click(play.center(), Modifiers::none());
		cx.run_until_parked();

		let requested = cx.read(|app| {
			host.read(app)
				.events
				.iter()
				.any(|e| matches!(e, ViewerEvent::PlayRequested { control: 1 }))
		});
		assert!(requested, "expected a PlayRequested event");
	}

	#[test]
	fn timecode_formatting_reuses_timeline() {
		let frame = Frame(3000);
		let text = format_timecode(frame, FrameRate::new(30, 1), TimeDisplay::Timecode);
		assert_eq!(text, "00:01:40:00");
	}

	#[gpui::test]
	async fn cpu_frame_source_renders_without_a_platform_surface(cx: &mut TestAppContext) {
		// The CPU-frame path (for non-macOS platforms without CVPixelBuffer)
		// accepts raw BGRA8 bytes in a RenderImage and renders through the
		// sprite atlas — no SurfaceSource involved.
		use gpui::RenderImage;
		use image::{Frame, RgbaImage};

		// A 2x2 opaque red frame, converted RGBA -> BGRA as gpui expects.
		let mut rgba = RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
		for pixel in rgba.chunks_exact_mut(4) {
			pixel.swap(0, 2);
		}
		let frame = Arc::new(RenderImage::new(smallvec::SmallVec::from_elem(
			Frame::new(rgba),
			1,
		)));

		let (cx, host) = make_host(cx);
		cx.update(|window, app| {
			host.read(app)
				.viewer
				.clone()
				.update(app, |viewer, cx| viewer.set_cpu_frame(Some(frame), cx));
			window.draw(app);
		});
		cx.run_until_parked();

		let is_cpu = cx.read(|app| {
			matches!(
				host.read(app).viewer.read(app).frame_source,
				Some(ViewerFrameSource::CpuFrame(_))
			)
		});
		assert!(is_cpu, "the frame source should be the CPU-frame variant");

		// Clearing the CPU frame falls back to the placeholder.
		cx.update(|window, app| {
			host.read(app)
				.viewer
				.clone()
				.update(app, |viewer, cx| viewer.set_cpu_frame(None, cx));
			window.draw(app);
		});
		cx.run_until_parked();
		let is_none = cx.read(|app| host.read(app).viewer.read(app).frame_source.is_none());
		assert!(is_none);
	}

	fn make_host(cx: &mut TestAppContext) -> (&'static mut VisualTestContext, Entity<Host>) {
		cx.update(|cx| cx.init_colors());
		let window = cx.open_window(size(px(640.0), px(420.0)), |window, cx| {
			let clock = cx.new(|_| MockClock {
				frame: Frame(0),
				playing: false,
			});
			let viewer = cx.new(|cx| ViewerWidget::new(1, clock, window, cx));
			let host = Host {
				viewer,
				events: Vec::new(),
			};
			cx.subscribe(
				&host.viewer,
				|host: &mut Host,
				 _v: Entity<ViewerWidget<MockClock>>,
				 event: &ViewerEvent,
				 _cx: &mut Context<Host>| {
					host.events.push(event.clone());
				},
			)
			.detach();
			host
		});
		cx.run_until_parked();
		let host = window.root(cx).unwrap();
		let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
		(cx, host)
	}

	#[gpui::test]
	async fn step_button_emits_step_request(cx: &mut TestAppContext) {
		let (cx, host) = make_host(cx);
		let step = cx
			.debug_bounds("gpui-widgets-viewer-step-forward")
			.expect("step button rendered");
		cx.simulate_click(step.center(), Modifiers::none());
		cx.run_until_parked();

		let requested = cx.read(|app| {
			host.read(app)
				.events
				.iter()
				.any(|e| matches!(e, ViewerEvent::StepRequested { delta: 1, .. }))
		});
		assert!(requested, "expected a StepRequested(+1) event");
	}

	#[gpui::test]
	async fn safe_frame_toggle_emits_and_switches(cx: &mut TestAppContext) {
		let (cx, host) = make_host(cx);
		let toggle = cx
			.debug_bounds("gpui-widgets-viewer-safe")
			.expect("safe-frame button rendered");
		cx.simulate_click(toggle.center(), Modifiers::none());
		cx.run_until_parked();

		let (requested, shown) = cx.read(|app| {
			let host = host.read(app);
			(
				host.events
					.iter()
					.any(|e| matches!(e, ViewerEvent::ToggleSafeFramesRequested { .. })),
				host.viewer.read(app).safe_margins,
			)
		});
		assert!(requested, "expected a ToggleSafeFramesRequested event");
		assert_eq!(shown, SafeMargins::On, "safe margins should now be shown");
	}

	#[test]
	fn zoom_cycles_fit_through_levels() {
		// The toolbar zoom button cycles Fit → 100% → 200% → Fit.
		assert_eq!(ViewerZoom::Fit.next(), ViewerZoom::Level(4));
		assert_eq!(ViewerZoom::Level(4).next(), ViewerZoom::Level(5));
		assert_eq!(ViewerZoom::Level(3).next(), ViewerZoom::Level(7));
		assert_eq!(ViewerZoom::Level(7).next(), ViewerZoom::Level(8));
		assert_eq!(
			ViewerZoom::Level(VIEWER_ZOOM_LEVELS.len() - 1).next(),
			ViewerZoom::Fit
		);
	}

	#[test]
	fn safe_margins_cycle_and_ratio() {
		assert_eq!(SafeMargins::Off.next(), SafeMargins::On);
		assert_eq!(SafeMargins::On.next(), SafeMargins::Off);
		assert_eq!(SafeMargins::Custom(0.5, 0.5).next(), SafeMargins::Off);
		assert_eq!(SafeMargins::Off.ratio(), None);
		assert_eq!(SafeMargins::On.ratio(), Some((0.9, 0.8)));
		assert_eq!(SafeMargins::Custom(0.7, 0.6).ratio(), Some((0.7, 0.6)));
	}

	#[test]
	fn waveform_mode_config_round_trips() {
		assert_eq!(WaveformMode::Automatic.config_value(), 0);
		assert_eq!(WaveformMode::Only.config_value(), 1);
		assert_eq!(WaveformMode::Both.config_value(), 2);
		assert_eq!(WaveformMode::from_config_value(0), WaveformMode::Automatic);
		assert_eq!(WaveformMode::from_config_value(1), WaveformMode::Only);
		assert_eq!(WaveformMode::from_config_value(2), WaveformMode::Both);
		// Out-of-range values clamp to the nearest valid mode.
		assert_eq!(WaveformMode::from_config_value(-5), WaveformMode::Automatic);
		assert_eq!(WaveformMode::from_config_value(9), WaveformMode::Both);
	}

	#[test]
	fn fps_counter_smooths_measured_rate() {
		let mut counter = FpsCounter::new();
		let t0 = std::time::Instant::now();
		counter.record(t0);
		assert_eq!(counter.value(), 0.0, "no rate before a second sample");
		// 100ms apart => 10 fps instant.
		counter.record(t0 + std::time::Duration::from_millis(100));
		assert!((counter.value() - 10.0).abs() < 1.0);
		counter.record(t0 + std::time::Duration::from_millis(200));
		assert!((counter.value() - 10.0).abs() < 1.5);
	}
}
