//! A theme demo: toggle between Olive Dark and Olive Light at runtime. Every
//! widget that reads `cx.default_colors()` re-themes immediately.

use gpui::{
    App, Bounds, ClickEvent, Context, Entity, Render, Window, WindowBounds, WindowOptions,
    colors::DefaultColors, div, prelude::*, px, size,
};
use gpui_widgets::checkbox::{CheckBox, CheckBoxEvent, CheckState};
use gpui_widgets::slider::{Slider, SliderEvent, SliderModel};
use gpui_widgets::theme::{OakTheme, apply_theme};
use gpui_widgets::value::ValueKind;

struct Example {
    slider: Entity<Slider>,
    checkbox: Entity<CheckBox>,
    dark: bool,
}

impl Example {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let slider = cx.new(|cx| {
            Slider::new(
                1,
                SliderModel::new(ValueKind::Float, 0.0, 1.0, 0.05, 0.5),
                window,
                cx,
            )
        });
        cx.subscribe(
            &slider,
            |_this: &mut Self, _s: Entity<Slider>, event: &SliderEvent, _cx| {
                println!("slider: {event:?}");
            },
        )
        .detach();

        let checkbox = cx.new(|cx| {
            CheckBox::new(2, CheckState::Checked, window, cx).with_label("Track enabled")
        });
        cx.subscribe(
            &checkbox,
            |_this: &mut Self, _c: Entity<CheckBox>, event: &CheckBoxEvent, _cx| {
                println!("checkbox: {event:?}");
            },
        )
        .detach();

        Self {
            slider,
            checkbox,
            dark: true,
        }
    }
}

impl Render for Example {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors().clone();
        let theme = if self.dark {
            OakTheme::olive_dark()
        } else {
            OakTheme::olive_light()
        };

        div()
            .size_full()
            .bg(theme.base)
            .flex()
            .flex_col()
            .gap_3()
            .p_6()
            .child(
                div()
                    .text_color(colors.text)
                    .child(format!("Current theme: {}", theme.name)),
            )
            .child(div().flex().items_center().gap_2().child(self.slider.clone()))
            .child(self.checkbox.clone())
            .child(
                div()
                    .id("theme-toggle")
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(colors.selected)
                    .text_color(colors.selected_text)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        this.dark = !this.dark;
                        let theme = if this.dark {
                            OakTheme::olive_dark()
                        } else {
                            OakTheme::olive_light()
                        };
                        apply_theme(cx, &theme);
                        cx.notify();
                    }))
                    .child("Toggle theme"),
            )
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        apply_theme(cx, &OakTheme::olive_dark());
        let bounds = Bounds::centered(None, size(px(480.0), px(320.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Example::new(window, cx)),
        )
        .expect("Failed to open window");

        cx.activate(true);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
    });
}
