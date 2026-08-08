//! A checkbox control with an optional tri-state (`Indeterminate`) state.
//!
//! The widget is request-only: clicking emits [`CheckBoxEvent::Toggled`]
//! with the state the control *would* move to; the host applies it through
//! its model and calls [`CheckBox::set_state`] (which also repaints) when it
//! accepts. The widget never changes its own state on click.

use gpui::{
    App, Bounds, ClickEvent, Context, ElementId, EventEmitter, FocusHandle, Focusable,
    Hsla, KeyDownEvent, Pixels, Render, Window, canvas, colors::DefaultColors, div, fill, point,
    prelude::*, px, size,
};
use gpui::PathBuilder;

/// The display state of a checkbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    /// The box is empty.
    Unchecked,
    /// The box is filled with a check mark.
    Checked,
    /// The box shows a horizontal bar (partially checked).
    Indeterminate,
}

impl CheckState {
    /// The next state after a click.
    ///
    /// With tri-state enabled the cycle is
    /// `Unchecked -> Checked -> Indeterminate -> Unchecked`; otherwise
    /// `Unchecked <-> Checked`.
    pub fn toggled(self, tri_state: bool) -> Self {
        match (self, tri_state) {
            (CheckState::Unchecked, _) => CheckState::Checked,
            (CheckState::Checked, true) => CheckState::Indeterminate,
            (CheckState::Checked, false) => CheckState::Unchecked,
            (CheckState::Indeterminate, _) => CheckState::Unchecked,
        }
    }
}

/// A request emitted when a checkbox is toggled.
#[derive(Debug, Clone, PartialEq)]
pub enum CheckBoxEvent {
    /// The user clicked (or pressed space/enter on) the box.
    Toggled {
        /// The control's stable id.
        control: usize,
        /// The state the control should move to.
        state: CheckState,
    },
}

/// A single checkbox row (box + optional label).
pub struct CheckBox {
    control: usize,
    state: CheckState,
    label: Option<gpui::SharedString>,
    enabled: bool,
    tri_state: bool,
    focus_handle: FocusHandle,
}

impl CheckBox {
    /// Create a checkbox for `control` in `state`.
    pub fn new(
        control: usize,
        state: CheckState,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            control,
            state,
            label: None,
            enabled: true,
            tri_state: false,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Attach a label shown to the right of the box.
    pub fn with_label(mut self, label: impl Into<gpui::SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Enable the tri-state cycle.
    pub fn with_tri_state(mut self, tri_state: bool) -> Self {
        self.tri_state = tri_state;
        self
    }

    /// Enable or disable the control (disabled boxes ignore clicks).
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// The current display state.
    pub fn state(&self) -> CheckState {
        self.state
    }

    /// Apply a new display state (from the host) and repaint.
    pub fn set_state(&mut self, state: CheckState, cx: &mut Context<Self>) {
        if self.state != state {
            self.state = state;
            cx.notify();
        }
    }

    fn emit_toggle(&self, cx: &mut Context<Self>) {
        if !self.enabled {
            return;
        }
        cx.emit(CheckBoxEvent::Toggled {
            control: self.control,
            state: self.state.toggled(self.tri_state),
        });
        cx.notify();
    }
}

impl EventEmitter<CheckBoxEvent> for CheckBox {}

impl Focusable for CheckBox {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for CheckBox {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors().clone();
        let state = self.state;
        let enabled = self.enabled;

        let mut box_el = div()
            .id(ElementId::named_usize("gpui-widgets-checkbox", self.control))
            .size(px(18.0))
            .rounded(px(4.0))
            .border_1()
            .border_color(if enabled { colors.border } else { colors.disabled })
            .bg(if state == CheckState::Checked {
                colors.selected
            } else {
                colors.background
            })
            .track_focus(&self.focus_handle)
            .cursor_pointer()
            .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                this.emit_toggle(cx);
                cx.stop_propagation();
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                if matches!(event.keystroke.key.as_str(), "space" | "enter") {
                    this.emit_toggle(cx);
                }
            }))
            .child(canvas(
                move |_bounds, _window, _cx| (),
                move |bounds, (), window, cx| {
                    paint_check(bounds, state, enabled, window, cx);
                },
            ));

        if !enabled {
            box_el = box_el.opacity(0.45);
        }

        let mut row = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(box_el);
        if let Some(label) = self.label.clone() {
            row = row.child(
                div()
                    .text_color(if enabled { colors.text } else { colors.disabled })
                    .child(label),
            );
        }
        row
    }
}

fn paint_check(
    bounds: Bounds<Pixels>,
    state: CheckState,
    enabled: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let colors = cx.default_colors().clone();
    let stroke_color = if enabled { Hsla::from(colors.selected_text) } else { Hsla::from(colors.disabled) };
    let mid_y = bounds.center().y;

    match state {
        CheckState::Unchecked => {}
        CheckState::Indeterminate => {
            // A centered horizontal bar.
            let bar = Bounds::new(
                point(bounds.left() + px(3.0), mid_y - px(1.0)),
                size(bounds.size.width - px(6.0), px(2.0)),
            );
            window.paint_quad(fill(bar, stroke_color));
        }
        CheckState::Checked => {
            let mut check = PathBuilder::stroke(px(2.0));
            check.move_to(point(bounds.left() + px(4.0), mid_y));
            check.line_to(point(bounds.left() + px(8.0), bounds.bottom() - px(4.0)));
            check.line_to(point(bounds.right() - px(3.0), bounds.top() + px(4.0)));
            if let Ok(path) = check.build() {
                window.paint_path(path, stroke_color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};

    #[test]
    fn binary_toggle_cycles() {
        assert_eq!(CheckState::Unchecked.toggled(false), CheckState::Checked);
        assert_eq!(CheckState::Checked.toggled(false), CheckState::Unchecked);
    }

    #[test]
    fn tri_state_toggle_cycles() {
        assert_eq!(CheckState::Unchecked.toggled(true), CheckState::Checked);
        assert_eq!(CheckState::Checked.toggled(true), CheckState::Indeterminate);
        assert_eq!(CheckState::Indeterminate.toggled(true), CheckState::Unchecked);
    }

    #[test]
    fn states_are_distinct() {
        assert_ne!(CheckState::Unchecked, CheckState::Checked);
        assert_ne!(CheckState::Checked, CheckState::Indeterminate);
    }

    #[gpui::test]
    async fn click_emits_toggle_request(cx: &mut TestAppContext) {

        struct Host {
            checkbox: Entity<CheckBox>,
            events: Vec<CheckBoxEvent>,
        }
        impl Render for Host {
            fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
                div().size_full().child(self.checkbox.clone())
            }
        }

        // `default_colors()` requires the global (not initialized in tests).
        cx.update(|cx| cx.init_colors());
        let window = cx.open_window(size(px(200.0), px(60.0)), |window, cx| {
            let checkbox = cx.new(|cx| {
                CheckBox::new(1, CheckState::Unchecked, window, cx).with_label("Mute")
            });            let host = Host {
                checkbox,
                events: Vec::new(),
            };
            cx.subscribe(
                &host.checkbox,
                |host: &mut Host,
                 _c: Entity<CheckBox>,
                 event: &CheckBoxEvent,
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
        cx.simulate_click(point(px(9.0), px(9.0)), Modifiers::none());
        cx.run_until_parked();

        let (state, emitted) = cx.read(|app| {
            let host = host.read(app);
            (
                host.checkbox.read(app).state(),
                host.events.iter().any(|e| {
                    matches!(e, CheckBoxEvent::Toggled { control: 1, state: CheckState::Checked })
                }),
            )
        });
        // The widget does not mutate itself; the host must apply the request.
        assert_eq!(state, CheckState::Unchecked);
        assert!(emitted);
    }
}
