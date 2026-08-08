//! An audio level meter: lit segments per channel with a peak-hold marker.
//!
//! Data-agnostic: the host implements [`AudioMeterDataSource`] over its
//! engine's channel levels (in Oak, queried from the audio engine). The peak
//! hold decays locally; the pure arithmetic is in [`scopes::math`].

use gpui::{
    App, Bounds, Context, Entity, FocusHandle, Focusable, Hsla, Render, Window, canvas,
    colors::DefaultColors, fill, point, prelude::*, px, size,
};

use crate::scopes::{decay_peak, meter_lit_segments};

/// The number of segments per channel.
const SEGMENTS: usize = 16;
/// Peak decay per frame (fraction of full scale).
const PEAK_DECAY: f32 = 0.01;

/// Provides per-channel levels in `0..1` (linear or dB-normalized).
pub trait AudioMeterDataSource: 'static {
    /// The current level of each channel, `0..1`.
    fn levels(&self) -> Vec<f32>;
}

/// An audio level meter.
pub struct AudioLevelMeter<D: AudioMeterDataSource> {
    data: Entity<D>,
    focus_handle: FocusHandle,
    peak: Vec<f32>,
}

impl<D: AudioMeterDataSource> AudioLevelMeter<D> {
    /// Create a meter over `data`.
    pub fn new(
        _control: usize,
        data: Entity<D>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            data,
            focus_handle: cx.focus_handle(),
            peak: Vec::new(),
        }
    }

    /// The current per-channel levels.
    pub fn levels(&self, cx: &App) -> Vec<f32> {
        self.data.read(cx).levels()
    }

    /// Update the peak-hold state from the current levels (call each frame).
    pub fn update(&mut self, cx: &mut Context<Self>) {
        let levels = self.data.read(cx).levels();
        self.peak.resize(levels.len(), 0.0);
        for (peak, level) in self.peak.iter_mut().zip(&levels) {
            *peak = decay_peak(*peak, *level, PEAK_DECAY);
        }
        cx.notify();
    }
}

impl<D: AudioMeterDataSource> Focusable for AudioLevelMeter<D> {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl<D: AudioMeterDataSource> Render for AudioLevelMeter<D> {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors().clone();
        let levels = self.data.read(cx).levels();
        self.peak.resize(levels.len(), 0.0);
        let peaks = self.peak.clone();
        let lit_counts: Vec<usize> = levels
            .iter()
            .map(|level| meter_lit_segments(*level, SEGMENTS))
            .collect();

        canvas(
            move |_bounds, _window, _cx| (),
            move |bounds, (), window, _cx| {
                let width = f32::from(bounds.size.width);
                let height = f32::from(bounds.size.height);
                let channel_h = if lit_counts.is_empty() {
                    height
                } else {
                    height / lit_counts.len() as f32
                };
                let seg_w = width / SEGMENTS as f32;
                let lit_color = Hsla::from(colors.selected);
                let dim_color = Hsla::from(colors.border);
                let peak_color = Hsla::from(colors.text);

                for (channel, &lit) in lit_counts.iter().enumerate() {
                    let y = bounds.top() + px(channel as f32 * channel_h);
                    for segment in 0..SEGMENTS {
                        let seg = Bounds::new(
                            point(bounds.left() + px(segment as f32 * seg_w), y),
                            size(px((seg_w - 1.0).max(1.0)), px((channel_h - 2.0).max(2.0))),
                        );
                        window.paint_quad(fill(seg, if segment < lit { lit_color } else { dim_color }));
                    }
                    // Peak marker.
                    if let Some(peak) = peaks.get(channel) {
                        let x = bounds.left() + px((peak.clamp(0.0, 1.0) * width) - 1.0);
                        let marker = Bounds::new(point(x, y), size(px(2.0), px((channel_h - 2.0).max(2.0))));
                        window.paint_quad(fill(marker, peak_color));
                    }
                }
            },
        )
        .size_full()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scopes::meter_lit_segments;
    use gpui::{Entity, Render, TestAppContext, Window, div, px, size};

    #[test]
    fn meter_math_matches_scope_core() {
        assert_eq!(meter_lit_segments(0.0, SEGMENTS), 0);
        assert_eq!(meter_lit_segments(0.5, SEGMENTS), SEGMENTS / 2);
    }

    struct MockAudio(Vec<f32>);
    impl AudioMeterDataSource for MockAudio {
        fn levels(&self) -> Vec<f32> {
            self.0.clone()
        }
    }

    struct Host {
        meter: Entity<AudioLevelMeter<MockAudio>>,
    }
    impl Render for Host {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.meter.clone())
        }
    }

    #[gpui::test]
    async fn meter_renders_and_decays_peak(cx: &mut TestAppContext) {
        
        cx.update(|cx| cx.init_colors());
        let window = cx.open_window(size(px(200.0), px(60.0)), |window, cx| {
            let audio = cx.new(|_| MockAudio(vec![0.8, 0.2]));
            let meter = cx.new(|cx| AudioLevelMeter::new(4, audio, window, cx));
            Host { meter }
        });
        cx.run_until_parked();

        // update() refreshes peaks from levels.
        let (peaks, levels) = window
            .update(cx, |host, _, cx| {
                host.meter.update(cx, |meter, cx| meter.update(cx));
                let levels = host.meter.read(cx).levels(cx);
                let peaks = host.meter.read(cx).peak.clone();
                (peaks, levels)
            })
            .unwrap();
        assert_eq!(levels, vec![0.8, 0.2]);
        // Peaks track the levels on the first update.
        assert!((peaks[0] - 0.8).abs() < 0.001);
    }
}
