//! A scopes demo: mock signal sources drive a histogram, waveform,
//! vectorscope and audio level meter. Every frame the mock data updates and
//! the scopes repaint.
//!
//! Run with `cargo run -p gpui_widgets --example scopes`.

use gpui::{
    App, Bounds, Context, Entity, Render, Window, WindowBounds, WindowOptions, div, prelude::*,
    px, size,
};
use gpui_widgets::audio_meter::{AudioLevelMeter, AudioMeterDataSource};
use gpui_widgets::scopes::{ChromaDataSource, Histogram, LumaDataSource, Vectorscope, Waveform};

struct MockLuma {
    frame: u64,
}
impl LumaDataSource for MockLuma {
    fn luma_samples(&self) -> Vec<f32> {
        // A moving gradient + noise-ish bars.
        (0..4096)
            .map(|i| {
                let x = i as f32 / 4096.0;
                let t = self.frame as f32 / 60.0;
                ((x + t * 0.25).fract() * 0.8 + 0.1).clamp(0.0, 1.0)
            })
            .collect()
    }
}

struct MockChroma {
    frame: u64,
}
impl ChromaDataSource for MockChroma {
    fn chroma_samples(&self) -> Vec<(f32, f32)> {
        // A rotating ring in chroma space.
        let t = self.frame as f32 / 60.0;
        (0..2048)
            .map(|i| {
                let angle = i as f32 / 2048.0 * std::f32::consts::TAU;
                (
                    0.5 + 0.4 * (angle + t).cos(),
                    0.5 + 0.4 * (angle + t).sin(),
                )
            })
            .collect()
    }
}

struct MockAudio {
    frame: u64,
}
impl AudioMeterDataSource for MockAudio {
    fn levels(&self) -> Vec<f32> {
        let t = self.frame as f32 / 60.0;
        vec![
            (0.5 + 0.5 * (t * 2.0).sin()).clamp(0.0, 1.0),
            (0.5 + 0.5 * (t * 1.7).cos()).clamp(0.0, 1.0),
        ]
    }
}

struct Example {
    luma: Entity<MockLuma>,
    chroma: Entity<MockChroma>,
    audio: Entity<MockAudio>,
    histogram: Entity<Histogram<MockLuma>>,
    waveform: Entity<Waveform<MockLuma>>,
    vectorscope: Entity<Vectorscope<MockChroma>>,
    meter: Entity<AudioLevelMeter<MockAudio>>,
}

impl Example {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let luma = cx.new(|_| MockLuma { frame: 0 });
        let chroma = cx.new(|_| MockChroma { frame: 0 });
        let audio = cx.new(|_| MockAudio { frame: 0 });

        let histogram = cx.new(|cx| Histogram::new(1, luma.clone(), window, cx));
        let waveform = cx.new(|cx| Waveform::new(2, luma.clone(), window, cx));
        let vectorscope = cx.new(|cx| Vectorscope::new(3, chroma.clone(), window, cx));
        let meter = cx.new(|cx| AudioLevelMeter::new(4, audio.clone(), window, cx));

        let this = cx.weak_entity();
        window.spawn(cx, async move |cx: &mut gpui::AsyncWindowContext| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(16))
                    .await;
                let _ = cx.update(|_window, app| {
                    if let Some(this) = this.upgrade() {
                        this.update(app, |this, cx| this.tick(cx));
                    }
                });
            }
        })
        .detach();

        Self {
            luma,
            chroma,
            audio,
            histogram,
            waveform,
            vectorscope,
            meter,
        }
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        self.luma.update(cx, |luma, _| luma.frame += 1);
        self.chroma.update(cx, |chroma, _| chroma.frame += 1);
        self.audio.update(cx, |audio, _| audio.frame += 1);
        self.meter.update(cx, |meter, cx| meter.update(cx));
        cx.notify();
    }
}

impl Render for Example {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .child(div().h(px(120.0)).child(self.histogram.clone()))
            .child(div().h(px(120.0)).child(self.waveform.clone()))
            .child(div().h(px(140.0)).child(self.vectorscope.clone()))
            .child(div().h(px(60.0)).child(self.meter.clone()))
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        cx.init_colors();
        let bounds = Bounds::centered(None, size(px(520.0), px(560.0)), cx);
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
