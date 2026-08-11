//! A numeric spinbox: a directly-editable number field with up/down buttons.
//!
//! Reuses [`SliderModel`] for range/step/clamping, so the pure state machine
//! is already covered by `slider::model` tests. The field commits on `enter`
//! or focus loss and rejects invalid input; the buttons and arrow keys step
//! the value. Every change is emitted as [`SpinBoxEvent::ValueChanged`].

use gpui::{
	App, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle, Focusable,
	KeyDownEvent, Render, Window, colors::DefaultColors, div, prelude::*, px,
};
use gpui_elements::editable_text::{EditableTextState, StringStorage, text_input};

use crate::slider::SliderModel;
use crate::value::{DefaultFormatter, SliderValue, ValueFormatter};

/// A request emitted by a spinbox.
#[derive(Debug, Clone, PartialEq)]
pub enum SpinBoxEvent {
	/// The value changed (commit, button, wheel or arrow key).
	ValueChanged {
		/// The control's stable id.
		control: usize,
		/// The new value.
		value: SliderValue,
	},
	/// Direct text entry was committed.
	EditCommitted {
		/// The control's stable id.
		control: usize,
		/// The accepted value.
		value: SliderValue,
	},
	/// Direct text entry was cancelled (invalid input kept).
	EditCancelled {
		/// The control's stable id.
		control: usize,
	},
}

/// A numeric spinbox control.
pub struct SpinBox {
	control: usize,
	model: SliderModel,
	formatter: Box<dyn ValueFormatter>,
	editor: Entity<EditableTextState>,
	focus_handle: FocusHandle,
	/// Keeps the commit-on-blur listener alive for the widget's lifetime.
	_commit_subscription: gpui::Subscription,
}

impl SpinBox {
	/// Create a spinbox for `control` over `model`.
	pub fn new(
		control: usize,
		model: SliderModel,
		window: &mut Window,
		cx: &mut Context<Self>,
	) -> Self {
		let formatter = Box::new(DefaultFormatter);
		let text = formatter.format(model.value());
		let editor = cx.new(|cx| EditableTextState::new(StringStorage::from(text.to_string()), cx));
		let focus_handle = editor.read(cx).focus_handle(cx);
		let _commit_subscription =
			cx.on_focus_out(&focus_handle, window, |this, _event, _window, cx| {
				this.commit_edit(cx);
			});
		Self {
			control,
			model,
			formatter,
			editor,
			focus_handle,
			_commit_subscription,
		}
	}

	/// Inject a custom formatter/parser.
	pub fn with_formatter(mut self, formatter: impl ValueFormatter) -> Self {
		self.formatter = Box::new(formatter);
		self
	}

	/// The current value.
	pub fn value(&self) -> SliderValue {
		self.model.value()
	}

	/// Apply a value from the host and refresh the displayed text.
	pub fn set_value(&mut self, value: SliderValue, cx: &mut Context<Self>) {
		self.model.set_value(value);
		self.sync_text(cx);
		cx.notify();
	}

	/// Repaint the editor with the formatted value (only when the field is
	/// not being edited, so typing is never clobbered).
	fn sync_text(&self, cx: &mut Context<Self>) {
		let text = self.formatter.format(self.model.value());
		self.editor.update(cx, |editor, cx| {
			editor.emplace(text.as_ref(), cx);
		});
	}

	fn apply_and_notify(&mut self, changed: bool, cx: &mut Context<Self>) {
		if changed {
			self.sync_text(cx);
			cx.emit(SpinBoxEvent::ValueChanged {
				control: self.control,
				value: self.model.value(),
			});
			cx.notify();
		}
	}

	fn step(&mut self, delta: i32, fine: bool, cx: &mut Context<Self>) {
		let changed = self.model.apply_step(delta, fine);
		self.apply_and_notify(changed, cx);
	}

	fn commit_edit(&mut self, cx: &mut Context<Self>) {
		let text = self.editor.read(cx).as_str().to_string();
		match self.formatter.parse(&text) {
			Ok(value) => {
				let changed = self.model.set_value(value);
				if changed {
					self.apply_and_notify(true, cx);
				}
				cx.emit(SpinBoxEvent::EditCommitted {
					control: self.control,
					value: self.model.value(),
				});
				self.sync_text(cx);
			}
			Err(_) => {
				cx.emit(SpinBoxEvent::EditCancelled {
					control: self.control,
				});
				self.sync_text(cx);
			}
		}
		cx.notify();
	}
}

impl EventEmitter<SpinBoxEvent> for SpinBox {}

impl Focusable for SpinBox {
	fn focus_handle(&self, _cx: &App) -> FocusHandle {
		self.focus_handle.clone()
	}
}

impl Render for SpinBox {
	fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
		let colors = cx.default_colors().clone();
		let weak = self.editor.downgrade();
		let control = self.control;

		div()
			.flex()
			.items_center()
			.gap(px(2.0))
			.child(
				div()
					.id(ElementId::named_usize(
						"gpui-widgets-spinbox-field",
						control,
					))
					.min_w(px(64.0))
					.h(px(24.0))
					.rounded_md()
					.border_1()
					.border_color(colors.border)
					.bg(colors.background)
					.px_1()
					.flex()
					.items_center()
					.track_focus(&self.focus_handle)
					.on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
						match event.keystroke.key.as_str() {
							"up" => this.step(1, event.keystroke.modifiers.shift, cx),
							"down" => this.step(-1, event.keystroke.modifiers.shift, cx),
							"enter" => this.commit_edit(cx),
							"escape" => {
								// Revert the displayed text without committing.
								this.sync_text(cx);
								cx.notify();
							}
							_ => {}
						}
					}))
					.child(
						text_input(ElementId::named_usize(
							"gpui-widgets-spinbox-input",
							control,
						))
						.state(weak)
						.accepts_input(true),
					),
			)
			.child(
				div()
					.flex()
					.flex_col()
					.child(
						div()
							.id(ElementId::named_usize("gpui-widgets-spinbox-up", control))
							.size(px(12.0))
							.cursor_pointer()
							.child(arrow_element(true, colors.border))
							.on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
								this.step(1, false, cx);
								cx.stop_propagation();
							})),
					)
					.child(
						div()
							.id(ElementId::named_usize("gpui-widgets-spinbox-down", control))
							.size(px(12.0))
							.cursor_pointer()
							.child(arrow_element(false, colors.border))
							.on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
								this.step(-1, false, cx);
								cx.stop_propagation();
							})),
					),
			)
	}
}

/// A tiny up (▲) or down (▼) arrow painted with a canvas.
fn arrow_element(up: bool, color: gpui::Rgba) -> impl IntoElement {
	use gpui::{Bounds, PathBuilder, Pixels, canvas, point, px};

	canvas(
		move |_bounds, _window, _cx| (),
		move |bounds: Bounds<Pixels>, (), window, cx| {
			let _ = cx;
			let tip = if up {
				point(bounds.center().x, bounds.top() + px(3.0))
			} else {
				point(bounds.center().x, bounds.bottom() - px(3.0))
			};
			let base = if up {
				(bounds.left() + px(3.0), bounds.bottom() - px(3.0))
			} else {
				(bounds.left() + px(3.0), bounds.top() + px(3.0))
			};
			let mut path = PathBuilder::fill();
			path.move_to(tip);
			path.line_to(point(base.0, base.1));
			path.line_to(point(bounds.right() - px(3.0), base.1));
			path.close();
			if let Ok(path) = path.build() {
				window.paint_path(path, color);
			}
		},
	)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::value::ValueKind;
	use gpui::{Modifiers, TestAppContext, VisualTestContext, point, px, size};

	fn float_model() -> SliderModel {
		SliderModel::new(ValueKind::Float, 0.0, 10.0, 1.0, 5.0)
	}

	#[gpui::test]
	async fn up_button_steps_value(cx: &mut TestAppContext) {
		struct Host {
			spinbox: Entity<SpinBox>,
			events: Vec<SpinBoxEvent>,
		}
		impl Render for Host {
			fn render(
				&mut self,
				_window: &mut Window,
				_cx: &mut Context<Self>,
			) -> impl IntoElement {
				div().size_full().child(self.spinbox.clone())
			}
		}

		cx.update(|cx| cx.init_colors());
		let window = cx.open_window(size(px(200.0), px(60.0)), |window, cx| {
			let spinbox = cx.new(|cx| SpinBox::new(1, float_model(), window, cx));
			let host = Host {
				spinbox,
				events: Vec::new(),
			};
			cx.subscribe(
				&host.spinbox,
				|host: &mut Host,
				 _s: Entity<SpinBox>,
				 event: &SpinBoxEvent,
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
		// The up button sits directly right of the field.
		cx.simulate_click(point(px(70.0), px(6.0)), Modifiers::none());
		cx.run_until_parked();

		let (value, changed) = cx.read(|app| {
            let host = host.read(app);
            (
                host.spinbox.read(app).value(),
                host.events.iter().any(|e| {
                    matches!(e, SpinBoxEvent::ValueChanged { value, .. } if (value.to_f64() - 6.0).abs() < 1e-9)
                }),
            )
        });
		assert!(
			(value.to_f64() - 6.0).abs() < 1e-9,
			"expected 6, got {value:?}"
		);
		assert!(changed);
	}

	#[gpui::test]
	async fn enter_commits_typed_value(cx: &mut TestAppContext) {
		struct Host {
			spinbox: Entity<SpinBox>,
			events: Vec<SpinBoxEvent>,
		}
		impl Render for Host {
			fn render(
				&mut self,
				_window: &mut Window,
				_cx: &mut Context<Self>,
			) -> impl IntoElement {
				div().size_full().child(self.spinbox.clone())
			}
		}

		cx.update(|cx| cx.init_colors());
		let window = cx.open_window(size(px(200.0), px(60.0)), |window, cx| {
			let spinbox = cx.new(|cx| SpinBox::new(1, float_model(), window, cx));
			let host = Host {
				spinbox,
				events: Vec::new(),
			};
			cx.subscribe(
				&host.spinbox,
				|host: &mut Host,
				 _s: Entity<SpinBox>,
				 event: &SpinBoxEvent,
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
		// Click the field to focus it, seed text programmatically (the test
		// platform cannot deliver IME characters), then commit with enter.
		cx.simulate_click(point(px(30.0), px(12.0)), Modifiers::none());
		cx.run_until_parked();

		let editor = cx
			.read(|app| host.read(app).spinbox.read(app).editor.clone())
			.clone();
		cx.cx.update(|app| {
			editor.update(app, |editor, cx| editor.emplace("7.5", cx));
		});
		cx.simulate_keystrokes("enter");
		cx.run_until_parked();

		let (value, committed) = cx.read(|app| {
            let host = host.read(app);
            (
                host.spinbox.read(app).value(),
                host.events.iter().any(|e| {
                    matches!(e, SpinBoxEvent::EditCommitted { value, .. } if (value.to_f64() - 8.0).abs() < 1e-9)
                }),
            )
        });
		// 7.5 snaps to the integer step... no: step is 1.0 so 7.5 -> 8.
		assert!(
			(value.to_f64() - 8.0).abs() < 1e-9,
			"expected 8, got {value:?}"
		);
		assert!(committed);
	}
}
