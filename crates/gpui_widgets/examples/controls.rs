//! A parameter-panel demo of every form control in `gpui_widgets`:
//! sliders (float / rational / angle with keying diamonds), a spinbox, a
//! combo box, checkboxes, a radio group, a color picker and a curve editor.
//!
//! Every edit is printed to stdout as a *request* event - the host would
//! apply it through its engine instead.

use gpui::{
    App, Bounds, Context, Entity, Render, Window, WindowBounds, WindowOptions,
    colors::DefaultColors, div, prelude::*, px, size,
};
use gpui_widgets::checkbox::{CheckBox, CheckBoxEvent, CheckState};
use gpui_widgets::color::{ColorPicker, ColorPickerEvent};
use gpui_widgets::combo_box::{ComboBox, ComboBoxEvent, ComboBoxOption};
use gpui_widgets::curve_editor::{CurveEditor, CurveEditorEvent, CurvePoint, CurveVec2};
use gpui_widgets::keyable::KeyingState;
use gpui_widgets::radio_group::{RadioGroup, RadioGroupEvent, RadioOption};
use gpui_widgets::slider::{Slider, SliderEvent, SliderModel};
use gpui_widgets::spinbox::{SpinBox, SpinBoxEvent};
use gpui_widgets::value::ValueKind;

struct Example {
    exposure: Entity<Slider>,
    frame_rate: Entity<Slider>,
    shutter_angle: Entity<Slider>,
    iso: Entity<SpinBox>,
    format: Entity<ComboBox>,
    muted: Entity<CheckBox>,
    solo: Entity<CheckBox>,
    playback_mode: Entity<RadioGroup>,
    color: Entity<ColorPicker>,
    remap: Entity<CurveEditor>,
}

impl Example {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Float slider with a key at the current frame.
        let exposure = cx.new(|cx| {
            Slider::new(
                1,
                SliderModel::new(ValueKind::Float, -5.0, 5.0, 0.1, 0.0),
                window,
                cx,
            )
            .with_keying(KeyingState::AtCurrentFrame)
        });
        cx.subscribe(
            &exposure,
            |_this: &mut Self, _s: Entity<Slider>, event: &SliderEvent, _cx| {
                println!("exposure request: {event:?}");
            },
        )
        .detach();

        // Rational slider: 1/24 .. 24/24 in numerator steps.
        let frame_rate = cx.new(|cx| {
            Slider::new(
                2,
                SliderModel::new(ValueKind::Rational, 1.0, 24.0, 1.0, 24.0)
                    .with_rational_den(24),
                window,
                cx,
            )
            .with_keying(KeyingState::HasKey)
        });
        cx.subscribe(
            &frame_rate,
            |_this: &mut Self, _s: Entity<Slider>, event: &SliderEvent, _cx| {
                println!("frame rate request: {event:?}");
            },
        )
        .detach();

        // Angle slider (degrees).
        let shutter_angle = cx.new(|cx| {
            Slider::new(
                3,
                SliderModel::new(ValueKind::Angle, 0.0, 360.0, 1.0, 180.0),
                window,
                cx,
            )
            .with_keying(KeyingState::NoKey)
        });
        cx.subscribe(
            &shutter_angle,
            |_this: &mut Self, _s: Entity<Slider>, event: &SliderEvent, _cx| {
                println!("shutter request: {event:?}");
            },
        )
        .detach();

        let iso = cx.new(|cx| {
            SpinBox::new(
                4,
                SliderModel::new(ValueKind::Integer, 100.0, 12800.0, 100.0, 800.0),
                window,
                cx,
            )
        });
        cx.subscribe(
            &iso,
            |_this: &mut Self, _s: Entity<SpinBox>, event: &SpinBoxEvent, _cx| {
                println!("iso request: {event:?}");
            },
        )
        .detach();

        let format = cx.new(|cx| {
            ComboBox::new(
                5,
                vec![
                    ComboBoxOption::new(1, "Frame"),
                    ComboBoxOption::new(2, "Timecode"),
                    ComboBoxOption::new(3, "Frames"),
                ],
                window,
                cx,
            )
            .with_placeholder("Choose…")
        });
        cx.subscribe(
            &format,
            |_this: &mut Self, _s: Entity<ComboBox>, event: &ComboBoxEvent, _cx| {
                println!("format request: {event:?}");
            },
        )
        .detach();

        let muted = cx.new(|cx| {
            CheckBox::new(6, CheckState::Unchecked, window, cx).with_label("Mute")
        });
        cx.subscribe(
            &muted,
            |_this: &mut Self, _s: Entity<CheckBox>, event: &CheckBoxEvent, _cx| {
                println!("mute request: {event:?}");
            },
        )
        .detach();

        let solo = cx.new(|cx| {
            CheckBox::new(7, CheckState::Indeterminate, window, cx)
                .with_label("Solo")
                .with_tri_state(true)
        });
        cx.subscribe(
            &solo,
            |_this: &mut Self, _s: Entity<CheckBox>, event: &CheckBoxEvent, _cx| {
                println!("solo request: {event:?}");
            },
        )
        .detach();

        let playback_mode = cx.new(|cx| {
            RadioGroup::new(
                8,
                vec![
                    RadioOption::new(1, "Loop"),
                    RadioOption::new(2, "Ping-pong"),
                    RadioOption::new(3, "Once"),
                ],
                window,
                cx,
            )
        });
        cx.subscribe(
            &playback_mode,
            |_this: &mut Self, _s: Entity<RadioGroup>, event: &RadioGroupEvent, _cx| {
                println!("playback mode request: {event:?}");
            },
        )
        .detach();

        let color = cx.new(|cx| {
            ColorPicker::new(9, gpui::Rgba { r: 1.0, g: 0.4, b: 0.1, a: 1.0 }, window, cx)
        });
        cx.subscribe(
            &color,
            |_this: &mut Self, _s: Entity<ColorPicker>, event: &ColorPickerEvent, _cx| {
                println!("color request: {event:?}");
            },
        )
        .detach();

        let remap = cx.new(|cx| {
            CurveEditor::new(
                10,
                vec![
                    CurvePoint::with_handles(0.0, 0.0, CurveVec2::new(0.0, 0.5)),
                    CurvePoint::with_handles(1.0, 1.0, CurveVec2::new(0.0, -0.5)),
                ],
                window,
                cx,
            )
        });
        cx.subscribe(
            &remap,
            |_this: &mut Self, _s: Entity<CurveEditor>, event: &CurveEditorEvent, _cx| {
                println!("remap request: {event:?}");
            },
        )
        .detach();

        Self {
            exposure,
            frame_rate,
            shutter_angle,
            iso,
            format,
            muted,
            solo,
            playback_mode,
            color,
            remap,
        }
    }
}

impl Render for Example {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors().clone();
        div()
            .size_full()
            .bg(colors.background)
            .flex()
            .flex_col()
            .p_4()
            .gap_3()
            .id("example-scroll")
            .overflow_y_scroll()
            .child(section("Exposure (float)", self.exposure.clone()))
            .child(section("Frame rate (rational)", self.frame_rate.clone()))
            .child(section("Shutter (angle)", self.shutter_angle.clone()))
            .child(section("ISO (spinbox)", self.iso.clone()))
            .child(section("Time format (combo)", self.format.clone()))
            .child(section("Audio", self.muted.clone()))
            .child(section("Track", self.solo.clone()))
            .child(section("Playback mode", self.playback_mode.clone()))
            .child(section("Accent color", self.color.clone()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_color(colors.text).child("Time remap (curve)"))
                    .child(self.remap.clone()),
            )
    }
}

/// A labeled row used by the demo panel.
fn section(label: impl Into<gpui::SharedString>, widget: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(div().w(px(130.0)).child(label.into()))
        .child(widget)
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        cx.init_colors();
        let bounds = Bounds::centered(None, size(px(620.0), px(760.0)), cx);
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
