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
	AnyElement, App, AsyncWindowContext, ClickEvent, Context, Entity, EventEmitter, FocusHandle,
	Focusable, ObjectFit, Render, RenderImage, SharedString, SurfaceSource, Window,
	colors::DefaultColors, div, img, prelude::*, px, surface,
};
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
	show_safe_frames: bool,
	zoom: bool,
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
			show_safe_frames: false,
			zoom: false,
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

	fn poll_clock(&mut self, cx: &mut Context<Self>) {
		let clock = self.clock.read(cx);
		let frame = clock.current_frame();
		let playing = clock.is_playing();
		if frame != self.transport.frame || playing != self.transport.playing {
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
			});

		if let Some(source) = &self.frame_source {
			let fit = if self.zoom {
				ObjectFit::Cover
			} else {
				ObjectFit::Contain
			};
			let picture_element: AnyElement = match source {
				ViewerFrameSource::Surface(surface_source) => surface(surface_source.clone())
					.size_full()
					.object_fit(fit)
					.into_any(),
				ViewerFrameSource::CpuFrame(image) => img(image.clone())
					.size_full()
					.min_w_0()
					.min_h_0()
					.object_fit(fit)
					.into_any(),
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

		if self.show_safe_frames {
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
							.w(px(560.0))
							.h(px(315.0))
							.border_1()
							.border_color(colors.selected),
					),
			);
		}

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
			.child(transport_button(
				"gpui-widgets-viewer-in",
				in_icon,
				"⏮",
				crate::i18n::tr("viewer.in_point", "入点"),
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
				cx.listener(|this, _event: &ClickEvent, _window, cx| {
					this.emit(
						ViewerEvent::ClearRangeRequested {
							control: this.control,
						},
						cx,
					);
				}),
			))
			.child(div().px_2().text_xs().text_color(colors.text).child(timecode))
			.child(div().flex_1())
			.child(button(
				"gpui-widgets-viewer-safe",
				crate::i18n::tr("viewer.safe_frames", "安全框"),
				cx.listener(|this, _event: &ClickEvent, _window, cx| {
					this.show_safe_frames = !this.show_safe_frames;
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
				cx.listener(|this, _event: &ClickEvent, _window, cx| {
					this.zoom = !this.zoom;
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
/// `None` (no resolver registered, or no file for the name).
fn transport_button(
	id: &'static str,
	icon: Option<std::path::PathBuf>,
	fallback: impl IntoElement,
	tooltip: SharedString,
	on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
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
		.hover(|style| style.bg(gpui::colors::Colors::dark().selected))
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

/// A small labeled button.
fn button(
	id: &'static str,
	label: impl IntoElement,
	on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
	div()
		.id(id)
		.debug_selector(move || id.into())
		.px_2()
		.py_0p5()
		.rounded_md()
		.cursor_pointer()
		.hover(|style| style.bg(gpui::colors::Colors::dark().selected))
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
				host.viewer.read(app).show_safe_frames,
			)
		});
		assert!(requested, "expected a ToggleSafeFramesRequested event");
		assert!(shown, "safe frames should now be shown locally");
	}
}
