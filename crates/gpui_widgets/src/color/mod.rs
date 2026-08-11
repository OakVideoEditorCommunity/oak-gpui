//! Color picking: a swatch button that opens a popup picker.
//!
//! The picker shows an SV (saturation/value) square with a hue strip, plus
//! RGBA numeric inputs. Editing any of them emits
//! [`ColorPickerEvent::ColorChanged`]; the host applies the color and calls
//! [`ColorPicker::set_color`] to keep the widget in sync. The eyedropper is a
//! placeholder for now.

mod hsv;

use gpui::{
	Anchor, App, Bounds, ClickEvent, Context, DragMoveEvent, ElementId, Entity, EventEmitter,
	FocusHandle, Focusable, Hsla, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Point, Render,
	Rgba, Window, anchored, canvas, colors::DefaultColors, deferred, div, fill, linear_color_stop,
	linear_gradient, point, prelude::*, px, size,
};

pub use hsv::HsvColor;

use crate::slider::SliderModel;
use crate::spinbox::{SpinBox, SpinBoxEvent};
use crate::value::{SliderValue, ValueKind};

/// The size of the SV square and hue strip in the picker popup.
const SV_SIZE: f32 = 150.0;
const HUE_STRIP_WIDTH: f32 = 16.0;
const HUE_STRIP_SEGMENTS: usize = 48;

/// A request emitted by a color picker.
#[derive(Debug, Clone, PartialEq)]
pub enum ColorPickerEvent {
	/// The color changed (SV/hue drag, RGBA edit).
	ColorChanged {
		/// The control's stable id.
		control: usize,
		/// The new color.
		color: Rgba,
	},
	/// The popup was opened.
	MenuOpened {
		/// The control's stable id.
		control: usize,
	},
	/// The popup was closed.
	MenuClosed {
		/// The control's stable id.
		control: usize,
	},
}

/// Which RGBA channel a spinbox edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Channel {
	R,
	G,
	B,
	A,
}

/// Marker payloads for the SV-square and hue-strip drags.
#[derive(Clone, Copy, Debug)]
struct SvDrag;
#[derive(Clone, Copy, Debug)]
struct HueDrag;
#[derive(Clone, Copy, Debug)]
struct PickGhost;

impl Render for PickGhost {
	fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
		div().w(px(0.0)).h(px(0.0))
	}
}

/// A swatch button with a popup color picker.
pub struct ColorPicker {
	control: usize,
	hsv: HsvColor,
	alpha: u8,
	focus_handle: FocusHandle,
	open: bool,
	popup_position: Point<Pixels>,
	was_open_at_down: bool,
	/// Bounds of the SV square / hue strip, refreshed each frame so clicks
	/// can be converted to coordinates.
	sv_bounds: Bounds<Pixels>,
	hue_bounds: Bounds<Pixels>,
	r_spin: Entity<SpinBox>,
	g_spin: Entity<SpinBox>,
	b_spin: Entity<SpinBox>,
	a_spin: Entity<SpinBox>,
}

impl ColorPicker {
	/// Create a picker for `control` showing `color`.
	pub fn new(control: usize, color: Rgba, window: &mut Window, cx: &mut Context<Self>) -> Self {
		let hsv = HsvColor::from_rgb(color.r, color.g, color.b);
		let alpha = (color.a * 255.0).round().clamp(0.0, 255.0) as u8;
		let r_spin = Self::channel_spinbox(cx, window, Channel::R, (color.r * 255.0) as u8);
		let g_spin = Self::channel_spinbox(cx, window, Channel::G, (color.g * 255.0) as u8);
		let b_spin = Self::channel_spinbox(cx, window, Channel::B, (color.b * 255.0) as u8);
		let a_spin = Self::channel_spinbox(cx, window, Channel::A, alpha);
		Self {
			control,
			hsv,
			alpha,
			focus_handle: cx.focus_handle(),
			open: false,
			popup_position: Point::default(),
			was_open_at_down: false,
			sv_bounds: Bounds::default(),
			hue_bounds: Bounds::default(),
			r_spin,
			g_spin,
			b_spin,
			a_spin,
		}
	}

	fn channel_spinbox(
		cx: &mut Context<Self>,
		window: &mut Window,
		channel: Channel,
		value: u8,
	) -> Entity<SpinBox> {
		let model = SliderModel::new(ValueKind::Integer, 0.0, 255.0, 1.0, value as f64);
		let spin = cx.new(|cx| SpinBox::new(channel as usize * 1000 + 0, model, window, cx));
		cx.subscribe(
			&spin,
			move |this: &mut Self, _s: Entity<SpinBox>, event: &SpinBoxEvent, cx| {
				this.on_channel_edit(channel, event, cx);
			},
		)
		.detach();
		spin
	}

	/// The current color.
	pub fn color(&self) -> Rgba {
		let [r, g, b, a] = self.hsv.to_rgba_u8(self.alpha);
		Rgba {
			r: r as f32 / 255.0,
			g: g as f32 / 255.0,
			b: b as f32 / 255.0,
			a: a as f32 / 255.0,
		}
	}

	/// Whether the popup is open.
	pub fn is_open(&self) -> bool {
		self.open
	}

	/// Apply a color from the host and repaint (also syncs the RGBA fields).
	pub fn set_color(&mut self, color: Rgba, cx: &mut Context<Self>) {
		self.hsv = HsvColor::from_rgb(color.r, color.g, color.b);
		self.alpha = (color.a * 255.0).round().clamp(0.0, 255.0) as u8;
		self.sync_channels(cx);
		cx.notify();
	}

	/// Push the current channels into the RGBA spinboxes (no events emitted).
	fn sync_channels(&self, cx: &mut Context<Self>) {
		let [r, g, b, a] = self.hsv.to_rgba_u8(self.alpha);
		cx.update_entity(&self.r_spin, |spin, cx| {
			spin.set_value(SliderValue::Integer(r as i64), cx);
		});
		cx.update_entity(&self.g_spin, |spin, cx| {
			spin.set_value(SliderValue::Integer(g as i64), cx);
		});
		cx.update_entity(&self.b_spin, |spin, cx| {
			spin.set_value(SliderValue::Integer(b as i64), cx);
		});
		cx.update_entity(&self.a_spin, |spin, cx| {
			spin.set_value(SliderValue::Integer(a as i64), cx);
		});
	}

	fn on_channel_edit(&mut self, channel: Channel, event: &SpinBoxEvent, cx: &mut Context<Self>) {
		let value = match event {
			SpinBoxEvent::ValueChanged { value, .. }
			| SpinBoxEvent::EditCommitted { value, .. } => *value,
			SpinBoxEvent::EditCancelled { .. } => return,
		};
		let v = value.to_f64().round().clamp(0.0, 255.0) as u8;
		match channel {
			Channel::A => self.alpha = v,
			_ => {
				let [r, g, b, _] = self.hsv.to_rgba_u8(self.alpha);
				let (r, g, b) = match channel {
					Channel::R => (v, g, b),
					Channel::G => (r, v, b),
					Channel::B => (r, g, v),
					Channel::A => unreachable!(),
				};
				self.hsv = HsvColor::from_rgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
			}
		}
		self.emit_changed(cx);
	}

	fn set_hsv(&mut self, hsv: HsvColor, cx: &mut Context<Self>) {
		if self.hsv != hsv {
			self.hsv = hsv;
			self.sync_channels(cx);
			self.emit_changed(cx);
		}
	}

	fn emit_changed(&mut self, cx: &mut Context<Self>) {
		cx.emit(ColorPickerEvent::ColorChanged {
			control: self.control,
			color: self.color(),
		});
		cx.notify();
	}

	fn open_menu(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
		if !self.open {
			self.open = true;
			self.popup_position = position;
			cx.emit(ColorPickerEvent::MenuOpened {
				control: self.control,
			});
			cx.notify();
		}
	}

	fn close_menu(&mut self, cx: &mut Context<Self>) {
		if self.open {
			self.open = false;
			cx.emit(ColorPickerEvent::MenuClosed {
				control: self.control,
			});
			cx.notify();
		}
	}

	/// Pick a color from the SV square at a window-space position.
	fn pick_sv(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
		let bounds = self.sv_bounds;
		if f32::from(bounds.size.width) <= 0.0 {
			return;
		}
		let x = ((f32::from(position.x) - f32::from(bounds.left())) / f32::from(bounds.size.width))
			.clamp(0.0, 1.0);
		let y = ((f32::from(position.y) - f32::from(bounds.top())) / f32::from(bounds.size.height))
			.clamp(0.0, 1.0);
		self.set_hsv(HsvColor::new(self.hsv.h, x, 1.0 - y), cx);
	}

	fn pick_hue(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
		let bounds = self.hue_bounds;
		if f32::from(bounds.size.height) <= 0.0 {
			return;
		}
		let y = ((f32::from(position.y) - f32::from(bounds.top())) / f32::from(bounds.size.height))
			.clamp(0.0, 1.0);
		self.set_hsv(HsvColor::new(y * 360.0, self.hsv.s, self.hsv.v), cx);
	}

	fn on_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
		match event.keystroke.key.as_str() {
			"escape" => self.close_menu(cx),
			"enter" | "space" => {
				if self.open {
					self.close_menu(cx);
				} else {
					self.open_menu(self.popup_position, cx);
				}
			}
			_ => {}
		}
	}
}

impl EventEmitter<ColorPickerEvent> for ColorPicker {}

impl Focusable for ColorPicker {
	fn focus_handle(&self, _cx: &App) -> FocusHandle {
		self.focus_handle.clone()
	}
}

impl Render for ColorPicker {
	fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
		let colors = cx.default_colors().clone();
		let entity = cx.entity();
		let hue_entity = entity.clone();
		let control = self.control;
		let color_hsla = hsv_to_hsla(self.hsv);
		let hue = self.hsv.h;

		// Swatch field.
		let swatch = div()
			.id(ElementId::named_usize("gpui-widgets-color-swatch", control))
			.w(px(28.0))
			.h(px(28.0))
			.rounded_md()
			.border_1()
			.border_color(if self.open {
				colors.selected
			} else {
				colors.border
			})
			.bg(color_hsla)
			.cursor_pointer()
			.track_focus(&self.focus_handle)
			.on_mouse_down(
				MouseButton::Left,
				cx.listener(|this, _event: &MouseDownEvent, _window, _cx| {
					this.was_open_at_down = this.open;
				}),
			)
			.on_click(cx.listener(|this, event: &ClickEvent, _window, cx| {
				if this.was_open_at_down {
					this.close_menu(cx);
				} else {
					this.open_menu(event.position(), cx);
				}
				cx.stop_propagation();
			}))
			.on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
				this.on_key_down(event, cx);
			}));

		// SV square + hue strip.
		let sv_square = div()
			.id(ElementId::named_usize("gpui-widgets-color-sv", control))
			.w(px(SV_SIZE))
			.h(px(SV_SIZE))
			.cursor_crosshair()
			.debug_selector(|| "color-sv".into())
			.on_mouse_down(
				MouseButton::Left,
				cx.listener(|this, event: &MouseDownEvent, _window, cx| {
					this.pick_sv(event.position, cx);
				}),
			)
			.on_drag(SvDrag, pick_ghost)
			.on_drag_move(
				cx.listener(|this, event: &DragMoveEvent<SvDrag>, _window, cx| {
					this.pick_sv(event.event.position, cx);
				}),
			)
			.child(
				canvas(
					move |bounds, _window, cx| {
						entity.update(cx, |this, _| this.sv_bounds = bounds);
						bounds
					},
					move |bounds, content, window, cx| {
						paint_sv_square(bounds, content, hue, window, cx);
					},
				)
				.size_full(),
			);

		let hue_strip = div()
			.id(ElementId::named_usize("gpui-widgets-color-hue", control))
			.w(px(HUE_STRIP_WIDTH))
			.h(px(SV_SIZE))
			.cursor_crosshair()
			.debug_selector(|| "color-hue".into())
			.on_mouse_down(
				MouseButton::Left,
				cx.listener(|this, event: &MouseDownEvent, _window, cx| {
					this.pick_hue(event.position, cx);
				}),
			)
			.on_drag(HueDrag, pick_ghost)
			.on_drag_move(
				cx.listener(|this, event: &DragMoveEvent<HueDrag>, _window, cx| {
					this.pick_hue(event.event.position, cx);
				}),
			)
			.child(
				canvas(
					move |bounds, _window, cx| {
						hue_entity.update(cx, |this, _| this.hue_bounds = bounds);
						bounds
					},
					move |bounds, content, window, cx| {
						paint_hue_strip(bounds, content, window, cx);
					},
				)
				.size_full(),
			);

		// The RGBA inputs + eyedropper placeholder.
		let rgba_row = div()
			.flex()
			.gap(px(4.0))
			.child(self.r_spin.clone())
			.child(self.g_spin.clone())
			.child(self.b_spin.clone())
			.child(self.a_spin.clone());

		let eyedropper = div()
			.id(ElementId::named_usize("gpui-widgets-color-eye", control))
			.px_2()
			.py_1()
			.rounded_md()
			.border_1()
			.border_color(colors.border)
			.text_color(colors.disabled)
			.opacity(0.7)
			.child("吸管（占位）");

		let popup = if self.open {
			deferred(
				anchored()
					.position(self.popup_position)
					.anchor(Anchor::TopLeft)
					.offset(point(px(0.0), px(32.0)))
					.snap_to_window_with_margin(px(8.0))
					.child(
						div()
							.p_2()
							.rounded_lg()
							.border_1()
							.border_color(colors.border)
							.bg(colors.container)
							.debug_selector(|| "color-popup".into())
							.flex()
							.flex_col()
							.gap(px(6.0))
							.on_mouse_up_out(
								MouseButton::Left,
								cx.listener(|this, _event: &gpui::MouseUpEvent, _window, cx| {
									this.close_menu(cx);
								}),
							)
							.child(
								div()
									.flex()
									.gap(px(4.0))
									.debug_selector(|| "color-row".into())
									.child(sv_square)
									.child(hue_strip),
							)
							.child(rgba_row)
							.child(eyedropper),
					),
			)
			.with_priority(1)
		} else {
			deferred(div())
		};

		div().relative().child(swatch).child(popup)
	}
}

/// Build the invisible ghost view for SV/hue drags.
fn pick_ghost(
	_drag: &impl Copy,
	_offset: Point<Pixels>,
	_window: &mut Window,
	cx: &mut App,
) -> Entity<PickGhost> {
	cx.new(|_| PickGhost)
}

/// Convert HSV to gpui's HSLA (lightness/saturation model).
fn hsv_to_hsla(hsv: HsvColor) -> Hsla {
	let h = hsv.h.rem_euclid(360.0) / 360.0;
	let l = hsv.v * (1.0 - hsv.s / 2.0);
	let s = if l <= 0.0 || l >= 1.0 {
		0.0
	} else {
		((hsv.v - l) / l.min(1.0 - l)).clamp(0.0, 1.0)
	};
	Hsla { h, s, l, a: 1.0 }
}

fn hue_hsla(h: f32) -> Hsla {
	Hsla {
		h: h.rem_euclid(360.0) / 360.0,
		s: 1.0,
		l: 0.5,
		a: 1.0,
	}
}

fn paint_sv_square(
	bounds: Bounds<Pixels>,
	_content: Bounds<Pixels>,
	hue: f32,
	window: &mut Window,
	cx: &mut App,
) {
	let _ = cx;
	// Base: pure hue.
	window.paint_quad(fill(bounds, hue_hsla(hue)));
	// White fade left -> right (saturation axis).
	let white = Hsla {
		h: 0.0,
		s: 0.0,
		l: 1.0,
		a: 1.0,
	};
	let transparent_white = Hsla {
		h: 0.0,
		s: 0.0,
		l: 1.0,
		a: 0.0,
	};
	window.paint_quad(fill(
		bounds,
		linear_gradient(
			90.0,
			linear_color_stop(white, 0.0),
			linear_color_stop(transparent_white, 1.0),
		),
	));
	// Black fade top -> bottom (value axis).
	let transparent_black = Hsla {
		h: 0.0,
		s: 0.0,
		l: 0.0,
		a: 0.0,
	};
	let black = Hsla {
		h: 0.0,
		s: 0.0,
		l: 0.0,
		a: 1.0,
	};
	window.paint_quad(fill(
		bounds,
		linear_gradient(
			180.0,
			linear_color_stop(transparent_black, 0.0),
			linear_color_stop(black, 1.0),
		),
	));
}

fn paint_hue_strip(
	bounds: Bounds<Pixels>,
	_content: Bounds<Pixels>,
	window: &mut Window,
	cx: &mut App,
) {
	let _ = cx;
	let height = f32::from(bounds.size.height);
	for i in 0..HUE_STRIP_SEGMENTS {
		let t0 = i as f32 / HUE_STRIP_SEGMENTS as f32;
		let t1 = (i + 1) as f32 / HUE_STRIP_SEGMENTS as f32;
		let hue = (t0 + t1) * 180.0;
		let segment = Bounds::new(
			point(bounds.left(), bounds.top() + px(t0 * height)),
			size(bounds.size.width, px((t1 - t0) * height)),
		);
		window.paint_quad(fill(segment, hue_hsla(hue)));
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use gpui::{Modifiers, TestAppContext, VisualTestContext};

	#[test]
	fn hsv_to_hsla_matches_rgb() {
		// Red: hsv(0,1,1) -> hsla(0,1,0.5) -> rgb(1,0,0).
		let hsla = hsv_to_hsla(HsvColor::new(0.0, 1.0, 1.0));
		assert!((hsla.h - 0.0).abs() < 0.001);
		assert!((hsla.s - 1.0).abs() < 0.001);
		assert!((hsla.l - 0.5).abs() < 0.001);
		// Gray: hsv(0,0,0.5) -> hsla(any,0,0.5).
		let gray = hsv_to_hsla(HsvColor::new(120.0, 0.0, 0.5));
		assert!(gray.s < 0.001);
		assert!((gray.l - 0.5).abs() < 0.001);
	}

	#[gpui::test]
	async fn clicking_sv_square_changes_color(cx: &mut TestAppContext) {
		struct Host {
			picker: Entity<ColorPicker>,
			events: Vec<ColorPickerEvent>,
		}
		impl Render for Host {
			fn render(
				&mut self,
				_window: &mut Window,
				_cx: &mut Context<Self>,
			) -> impl IntoElement {
				div().size_full().child(self.picker.clone())
			}
		}

		cx.update(|cx| cx.init_colors());
		let window = cx.open_window(size(px(300.0), px(300.0)), |window, cx| {
			let picker = cx.new(|cx| {
				ColorPicker::new(
					1,
					Rgba {
						r: 1.0,
						g: 0.0,
						b: 0.0,
						a: 1.0,
					},
					window,
					cx,
				)
			});
			let host = Host {
				picker,
				events: Vec::new(),
			};
			cx.subscribe(
				&host.picker,
				|host: &mut Host,
				 _p: Entity<ColorPicker>,
				 event: &ColorPickerEvent,
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
		// Open the picker by clicking the swatch.
		cx.simulate_click(point(px(14.0), px(14.0)), Modifiers::none());
		cx.run_until_parked();
		cx.update(|window, cx| {
			window.draw(cx).clear();
		});

		let sv_bounds = cx
			.debug_bounds("color-sv")
			.expect("SV square should be rendered");
		eprintln!(
			"bounds: sv={:?} row={:?} popup={:?}",
			cx.debug_bounds("color-sv"),
			cx.debug_bounds("color-row"),
			cx.debug_bounds("color-popup")
		);
		// Click near the top-left of the SV square: that is white.
		let click = point(sv_bounds.left() + px(10.0), sv_bounds.top() + px(10.0));
		cx.simulate_click(click, Modifiers::none());
		cx.run_until_parked();

		let (color, changed) = cx.read(|app| {
			let host = host.read(app);
			let color = host.picker.read(app).color();
			let changed = host
				.events
				.iter()
				.any(|e| matches!(e, ColorPickerEvent::ColorChanged { .. }));
			(color, changed)
		});
		assert!(changed, "expected a ColorChanged event");
		// Near the top-left the color is a pale, low-saturation tint of the
		// hue (high value, low saturation) - clearly not the initial red.
		assert!(
			color.r > 0.85 && color.g > 0.8 && color.b > 0.8,
			"got {color:?}"
		);
		assert!(
			color.g > color.b - 0.01 || (color.r - color.g).abs() > 0.05,
			"got {color:?}"
		);
	}
}
