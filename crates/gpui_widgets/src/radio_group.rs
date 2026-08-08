//! A radio button group: exactly one option selected at a time.
//!
//! Request-only like the other controls: clicking an option emits
//! [`RadioGroupEvent::Selected`]; the host applies the selection and updates
//! the widget via [`RadioGroup::set_selected`].

use gpui::{
    App, ClickEvent, Context, ElementId, EventEmitter, FocusHandle, Focusable, Render,
    SharedString, Window, colors::DefaultColors, div, prelude::*, px,
};

/// A single selectable option.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadioOption {
    /// The option's value (stable id).
    pub value: usize,
    /// The label shown next to the radio circle.
    pub label: SharedString,
}

impl RadioOption {
    /// Create an option.
    pub fn new(value: usize, label: impl Into<SharedString>) -> Self {
        Self {
            value,
            label: label.into(),
        }
    }
}

/// A request emitted when an option is selected.
#[derive(Debug, Clone, PartialEq)]
pub enum RadioGroupEvent {
    /// The user clicked an option.
    Selected {
        /// The group's stable id.
        control: usize,
        /// The chosen option value.
        value: usize,
    },
}

/// A group of mutually-exclusive radio options.
pub struct RadioGroup {
    control: usize,
    options: Vec<RadioOption>,
    selected: Option<usize>,
    enabled: bool,
    focus_handle: FocusHandle,
}

impl RadioGroup {
    /// Create a radio group for `control` over `options`.
    pub fn new(
        control: usize,
        options: Vec<RadioOption>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            control,
            options,
            selected: None,
            enabled: true,
            focus_handle: cx.focus_handle(),
        }
    }

    /// The currently selected value, if any.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Apply the host's selection and repaint.
    pub fn set_selected(&mut self, selected: Option<usize>, cx: &mut Context<Self>) {
        if self.selected != selected {
            self.selected = selected;
            cx.notify();
        }
    }

    /// Enable or disable the whole group.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    fn emit_select(&self, value: usize, cx: &mut Context<Self>) {
        if !self.enabled {
            return;
        }
        cx.emit(RadioGroupEvent::Selected {
            control: self.control,
            value,
        });
        cx.notify();
    }
}

impl EventEmitter<RadioGroupEvent> for RadioGroup {}

impl Focusable for RadioGroup {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RadioGroup {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors().clone();
        let selected = self.selected;
        let enabled = self.enabled;
        let control = self.control;

        let mut column = div().flex().flex_col().gap(px(4.0));
        for option in self.options.clone() {
            let is_selected = selected == Some(option.value);
            let accent = if is_selected {
                colors.selected
            } else {
                colors.background
            };
            let border = if enabled {
                colors.border
            } else {
                colors.disabled
            };
            let text_color = if enabled {
                colors.text
            } else {
                colors.disabled
            };
            let value = option.value;
            let label = option.label;

            column = column.child(
                div()
                    .id(ElementId::named_usize(
                        format!("gpui-widgets-radio-{control}"),
                        value,
                    ))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .cursor_pointer()
                    .track_focus(&self.focus_handle)
                    .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                        this.emit_select(value, cx);
                        cx.stop_propagation();
                    }))
                    .child(
                        div()
                            .size(px(16.0))
                            .rounded_full()
                            .border_1()
                            .border_color(border)
                            .bg(accent)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(if is_selected {
                                div().size(px(6.0)).rounded_full().bg(colors.selected_text)
                            } else {
                                div().size(px(0.0))
                            }),
                    )
                    .child(div().text_color(text_color).child(label)),
            );
        }
        column
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, point, px, size};

    #[test]
    fn option_construction() {
        let option = RadioOption::new(3, "1080p");
        assert_eq!(option.value, 3);
        assert_eq!(option.label, "1080p");
    }

    #[gpui::test]
    async fn click_emits_selection(cx: &mut TestAppContext) {
        struct Host {
            group: Entity<RadioGroup>,
            events: Vec<RadioGroupEvent>,
        }
        impl Render for Host {
            fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
                div().size_full().child(self.group.clone())
            }
        }

        cx.update(|cx| cx.init_colors());
        let window = cx.open_window(size(px(200.0), px(120.0)), |window, cx| {
            let group = cx.new(|cx| {
                RadioGroup::new(
                    1,
                    vec![
                        RadioOption::new(1, "1080p"),
                        RadioOption::new(2, "4K"),
                        RadioOption::new(3, "8K"),
                    ],
                    window,
                    cx,
                )
            });
            let host = Host {
                group,
                events: Vec::new(),
            };
            cx.subscribe(
                &host.group,
                |host: &mut Host,
                 _g: Entity<RadioGroup>,
                 event: &RadioGroupEvent,
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
        // Second option (4K) is around y = 30 + 4 + 16/2.
        cx.simulate_click(point(px(30.0), px(42.0)), Modifiers::none());
        cx.run_until_parked();

        let emitted = cx.read(|app| {
            host.read(app).events.iter().any(|e| {
                matches!(e, RadioGroupEvent::Selected { control: 1, value: 2 })
            })
        });
        assert!(emitted);
    }
}
