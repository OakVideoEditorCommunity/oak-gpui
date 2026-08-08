//! A combo box: a field showing the current selection with a drop-down menu.
//!
//! The menu is a pure-gpui popup built from `anchored` + `deferred` (no
//! platform primitives). Request-only: selecting an option emits
//! [`ComboBoxEvent::Selected`]; the host applies it and calls
//! [`ComboBox::set_selected`].
//!
//! Interactions: click the field to toggle the menu; click an option to pick
//! it; arrow keys (when the field is focused) cycle the selection; `escape`
//! or a click outside closes the menu.

use gpui::{
    Anchor, App, ClickEvent, Context, ElementId, EventEmitter, FocusHandle, Focusable, KeyDownEvent,
    Pixels, Point, Render, SharedString, Window, anchored, colors::DefaultColors, deferred, div,
    point, prelude::*, px,
};

/// A selectable option.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComboBoxOption {
    /// The option's value (stable id).
    pub value: usize,
    /// The label shown in the field and the menu.
    pub label: SharedString,
}

impl ComboBoxOption {
    /// Create an option.
    pub fn new(value: usize, label: impl Into<SharedString>) -> Self {
        Self {
            value,
            label: label.into(),
        }
    }
}

/// A request emitted by a combo box.
#[derive(Debug, Clone, PartialEq)]
pub enum ComboBoxEvent {
    /// The user picked an option (click or arrow key).
    Selected {
        /// The control's stable id.
        control: usize,
        /// The chosen option value.
        value: usize,
    },
    /// The menu was opened.
    MenuOpened {
        /// The control's stable id.
        control: usize,
    },
    /// The menu was closed.
    MenuClosed {
        /// The control's stable id.
        control: usize,
    },
}

/// Move `selected` by `delta` positions over a list of `len` options,
/// wrapping around. `None` moves to the first (or last) option.
pub fn cycle_selection(selected: Option<usize>, len: usize, delta: i32) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let current = match selected {
        Some(index) => index,
        None if delta > 0 => len - 1,
        None => 0,
    };
    Some(((current as i64 + delta as i64).rem_euclid(len as i64)) as usize)
}

/// A drop-down selection control.
pub struct ComboBox {
    control: usize,
    options: Vec<ComboBoxOption>,
    selected: Option<usize>,
    placeholder: Option<SharedString>,
    open: bool,
    /// Window position of the menu, recorded when the field is clicked.
    popup_position: Point<Pixels>,
    /// Whether the menu was open when the field was pressed, so the toggle on
    /// mouse-up does not reopen a menu that an outside-click close already
    /// dismissed on the same click.
    was_open_at_down: bool,
    focus_handle: FocusHandle,
}

impl ComboBox {
    /// Create a combo box for `control` over `options`.
    pub fn new(
        control: usize,
        options: Vec<ComboBoxOption>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            control,
            options,
            selected: None,
            placeholder: None,
            open: false,
            popup_position: Point::default(),
            was_open_at_down: false,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Text shown when nothing is selected.
    pub fn with_placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// The currently selected option value, if any.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Whether the menu is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Apply the host's selection and repaint.
    pub fn set_selected(&mut self, selected: Option<usize>, cx: &mut Context<Self>) {
        if self.selected != selected {
            self.selected = selected;
            cx.notify();
        }
    }

    fn selected_index(&self) -> Option<usize> {
        self.selected
            .and_then(|value| self.options.iter().position(|o| o.value == value))
    }

    fn open_menu(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.open {
            self.open = true;
            self.popup_position = position;
            cx.emit(ComboBoxEvent::MenuOpened {
                control: self.control,
            });
            cx.notify();
        }
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.open = false;
            cx.emit(ComboBoxEvent::MenuClosed {
                control: self.control,
            });
            cx.notify();
        }
    }

    fn toggle_menu(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.open {
            self.close_menu(cx);
        } else {
            self.open_menu(position, cx);
        }
    }

    fn pick(&mut self, value: usize, cx: &mut Context<Self>) {
        self.selected = Some(value);
        self.open = false;
        cx.emit(ComboBoxEvent::Selected {
            control: self.control,
            value,
        });
        cx.emit(ComboBoxEvent::MenuClosed {
            control: self.control,
        });
        cx.notify();
    }

    fn cycle(&mut self, delta: i32, cx: &mut Context<Self>) {
        if let Some(index) = cycle_selection(self.selected_index(), self.options.len(), delta) {
            let value = self.options[index].value;
            self.pick(value, cx);
        }
    }
}

impl EventEmitter<ComboBoxEvent> for ComboBox {}

impl Focusable for ComboBox {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ComboBox {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors().clone();
        let label = self
            .selected
            .and_then(|value| self.options.iter().find(|o| o.value == value))
            .map(|o| o.label.clone())
            .or_else(|| self.placeholder.clone())
            .unwrap_or_else(|| SharedString::from(""));
        let open = self.open;
        let control = self.control;

        let field = div()
            .id(ElementId::named_usize("gpui-widgets-combo-field", control))
            .min_w(px(140.0))
            .h(px(24.0))
            .rounded_md()
            .border_1()
            .border_color(if open { colors.selected } else { colors.border })
            .bg(colors.background)
            .px_2()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(6.0))
            .cursor_pointer()
            .track_focus(&self.focus_handle)
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _event: &gpui::MouseDownEvent, _window, _cx| {
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
                match event.keystroke.key.as_str() {
                    "up" => this.cycle(-1, cx),
                    "down" => this.cycle(1, cx),
                    "escape" => this.close_menu(cx),
                    "enter" | "space" => {
                        this.toggle_menu(this.popup_position, cx);
                    }
                    _ => {}
                }
            }))
            .child(div().text_color(colors.text).child(label))
            .child(div().child(chevron(colors.border)));

        let popup = if open {
            // Build the option list inline so each option can carry its own
            // view listener.
            let mut menu = div()
                .id(ElementId::named_usize("gpui-widgets-combo-menu", control))
                .debug_selector(|| "combo-menu".into())
                .min_w(px(140.0))
                .rounded_md()
                .border_1()
                .border_color(colors.border)
                .bg(colors.container)
                .py_1()
                .flex()
                .flex_col()
                .overflow_y_scroll()
                .max_h(px(220.0))
                .on_mouse_up_out(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _event: &gpui::MouseUpEvent, _window, cx| {
                        this.close_menu(cx);
                    }),
                );

            for option in &self.options {
                let is_selected = self.selected == Some(option.value);
                let value = option.value;
                let label = option.label.clone();
                menu = menu.child(
                    div()
                        .id(ElementId::named_usize(
                            format!("gpui-widgets-combo-option-{control}"),
                            value,
                        ))
                        .px_2()
                        .py_1()
                        .text_color(colors.text)
                        .hover(|style| style.bg(colors.selected))
                        .when(is_selected, |el| el.text_color(colors.selected_text))
                        .on_click(
                            cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                                this.pick(value, cx);
                                cx.stop_propagation();
                            }),
                        )
                        .child(label),
                );
            }

            deferred(
                anchored()
                    .position(self.popup_position)
                    .anchor(Anchor::TopLeft)
                    .offset(point(px(0.0), px(26.0)))
                    .snap_to_window_with_margin(px(8.0))
                    .child(menu),
            )
            .with_priority(1)
        } else {
            deferred(div())
        };

        div()
            .relative()
            .child(field)
            .child(popup)
    }
}

/// A small down-chevron painted with a canvas.
fn chevron(color: gpui::Rgba) -> impl IntoElement {
    use gpui::{canvas, point, px, Bounds, PathBuilder, Pixels};

    canvas(
        move |_bounds, _window, _cx| (),
        move |bounds: Bounds<Pixels>, (), window, cx| {
            let _ = cx;
            let mut path = PathBuilder::fill();
            path.move_to(point(bounds.left() + px(3.0), bounds.top() + px(2.0)));
            path.line_to(point(bounds.center().x, bounds.bottom() - px(2.0)));
            path.line_to(point(bounds.right() - px(3.0), bounds.top() + px(2.0)));
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
    use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, point, px, size};

    #[test]
    fn cycle_selection_wraps() {
        assert_eq!(cycle_selection(None, 3, 1), Some(0));
        assert_eq!(cycle_selection(None, 3, -1), Some(2));
        assert_eq!(cycle_selection(Some(0), 3, -1), Some(2));
        assert_eq!(cycle_selection(Some(2), 3, 1), Some(0));
        assert_eq!(cycle_selection(Some(1), 3, 1), Some(2));
        assert_eq!(cycle_selection(None, 0, 1), None);
    }

    fn options() -> Vec<ComboBoxOption> {
        vec![
            ComboBoxOption::new(1, "Frame"),
            ComboBoxOption::new(2, "Timecode"),
            ComboBoxOption::new(3, "Frames"),
        ]
    }

    #[gpui::test]
    async fn click_opens_menu_and_selects(cx: &mut TestAppContext) {
        struct Host {
            combo: Entity<ComboBox>,
            events: Vec<ComboBoxEvent>,
        }
        impl Render for Host {
            fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
                div().size_full().child(self.combo.clone())
            }
        }

        cx.update(|cx| cx.init_colors());
        let window = cx.open_window(size(px(300.0), px(200.0)), |window, cx| {
            let combo = cx.new(|cx| ComboBox::new(1, options(), window, cx));
            let host = Host {
                combo,
                events: Vec::new(),
            };
            cx.subscribe(
                &host.combo,
                |host: &mut Host,
                 _c: Entity<ComboBox>,
                 event: &ComboBoxEvent,
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
        // Open the menu, then force a redraw so the popup lands in the frame.
        cx.simulate_click(point(px(70.0), px(12.0)), Modifiers::none());
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.draw(cx).clear();
        });

        let menu_visible = cx.debug_bounds("combo-menu").is_some();
        eprintln!("menu bounds: {:?}", cx.debug_bounds("combo-menu"));
        assert!(menu_visible, "menu should be open after clicking the field");

        // Pick the second option (roughly below the field).
        // Pick the second option (menu at x=70, options from x=71; use a
        // well-inside point).
        cx.simulate_click(point(px(90.0), px(94.0)), Modifiers::none());
        cx.run_until_parked();
        let (selected, emitted, menu_closed) = cx.read(|app| {
            let host = host.read(app);
            (
                host.combo.read(app).selected(),
                host.events.iter().any(|e| {
                    matches!(e, ComboBoxEvent::Selected { control: 1, value: 2 })
                }),
                host.events
                    .iter()
                    .any(|e| matches!(e, ComboBoxEvent::MenuClosed { .. })),
            )
        });
        assert_eq!(selected, Some(2));
        assert!(emitted);
        assert!(menu_closed);
    }

    #[gpui::test]
    async fn arrow_keys_cycle_selection(cx: &mut TestAppContext) {
        struct Host {
            combo: Entity<ComboBox>,
            events: Vec<ComboBoxEvent>,
        }
        impl Render for Host {
            fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
                div().size_full().child(self.combo.clone())
            }
        }

        cx.update(|cx| cx.init_colors());
        let window = cx.open_window(size(px(300.0), px(200.0)), |window, cx| {
            let combo = cx.new(|cx| ComboBox::new(1, options(), window, cx));
            let host = Host {
                combo,
                events: Vec::new(),
            };
            cx.subscribe(
                &host.combo,
                |host: &mut Host,
                 _c: Entity<ComboBox>,
                 event: &ComboBoxEvent,
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
        // Focus the field, then press down twice: none -> first -> second.
        cx.simulate_click(point(px(70.0), px(12.0)), Modifiers::none());
        cx.run_until_parked();
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        cx.simulate_keystrokes("down");
        cx.run_until_parked();
        cx.simulate_keystrokes("down");
        cx.run_until_parked();

        let selected = cx.read(|app| host.read(app).combo.read(app).selected());
        assert_eq!(selected, Some(2));
    }
}
