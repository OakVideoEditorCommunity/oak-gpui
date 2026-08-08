//! Scope widgets: histogram, waveform and vectorscope, painted from host
//! frame samples.
//!
//! Data-agnostic: the host implements [`LumaDataSource`] / [`ChromaDataSource`]
//! over its frame buffers (in Oak, sampled from the renderer via C ABI). The
//! pure math lives in [`math`] and is unit-tested.

mod math;

pub use math::*;

use gpui::{
    App, Bounds, Context, Entity, FocusHandle, Focusable, Hsla, Render, Window, canvas,
    colors::DefaultColors, fill, point, prelude::*, px, size,
};

/// Provides luma samples (`0..1`) for the histogram and waveform scopes.
pub trait LumaDataSource: 'static {
    /// Luma samples for the current frame.
    fn luma_samples(&self) -> Vec<f32>;
}

/// Provides chroma samples for the vectorscope.
pub trait ChromaDataSource: 'static {
    /// `(u, v)` samples in `0..1` (centered on `0.5`).
    fn chroma_samples(&self) -> Vec<(f32, f32)>;
}

/// A luminance histogram.
pub struct Histogram<D: LumaDataSource> {
    data: Entity<D>,
    focus_handle: FocusHandle,
}

impl<D: LumaDataSource> Histogram<D> {
    /// Create a histogram over `data`.
    pub fn new(
        _control: usize,
        data: Entity<D>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            data,
            focus_handle: cx.focus_handle(),
        }
    }

    /// The current histogram bins (for tests and hosts).
    pub fn bins(&self, cx: &Context<Self>) -> Vec<u32> {
        histogram_bins(&self.data.read(cx).luma_samples(), 64)
    }
}

impl<D: LumaDataSource> Focusable for Histogram<D> {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl<D: LumaDataSource> Render for Histogram<D> {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors().clone();
        let bins = self.bins(cx);
        let height = bins.iter().copied().max().unwrap_or(1).max(1) as f32;
        canvas(
            move |_bounds, _window, _cx| (),
            move |bounds, (), window, _cx| {
                let width = f32::from(bounds.size.width);
                let bar_w = width / bins.len() as f32;
                let bar_color = Hsla::from(colors.selected);
                for (index, &count) in bins.iter().enumerate() {
                    let h = (count as f32 / height) * f32::from(bounds.size.height);
                    let bar = Bounds::new(
                        point(bounds.left() + px(index as f32 * bar_w), bounds.bottom() - px(h)),
                        size(px((bar_w - 1.0).max(1.0)), px(h)),
                    );
                    window.paint_quad(fill(bar, bar_color));
                }
            },
        )
        .size_full()
    }
}

/// A waveform scope (min/max envelope over the frame's luma).
pub struct Waveform<D: LumaDataSource> {
    data: Entity<D>,
    focus_handle: FocusHandle,
}

impl<D: LumaDataSource> Waveform<D> {
    /// Create a waveform scope.
    pub fn new(
        _control: usize,
        data: Entity<D>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            data,
            focus_handle: cx.focus_handle(),
        }
    }

    /// The current envelope columns.
    pub fn envelope(&self, cx: &Context<Self>) -> Vec<(f32, f32)> {
        waveform_envelope(&self.data.read(cx).luma_samples(), 128)
    }
}

impl<D: LumaDataSource> Focusable for Waveform<D> {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl<D: LumaDataSource> Render for Waveform<D> {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors().clone();
        let envelope = self.envelope(cx);
        canvas(
            move |_bounds, _window, _cx| (),
            move |bounds, (), window, _cx| {
                let width = f32::from(bounds.size.width);
                let height = f32::from(bounds.size.height);
                let col_w = width / envelope.len() as f32;
                let line = Hsla::from(colors.selected);
                for (column, &(min, max)) in envelope.iter().enumerate() {
                    let y_min = (1.0 - min.clamp(0.0, 1.0)) * height;
                    let y_max = (1.0 - max.clamp(0.0, 1.0)) * height;
                    let band = Bounds::new(
                        point(bounds.left() + px(column as f32 * col_w), bounds.top() + px(y_max)),
                        size(px((col_w - 0.5).max(0.5)), px((y_min - y_max).max(1.0))),
                    );
                    window.paint_quad(fill(band, line));
                }
            },
        )
        .size_full()
    }
}

/// A vectorscope (chroma projection with a graticule).
pub struct Vectorscope<D: ChromaDataSource> {
    data: Entity<D>,
    focus_handle: FocusHandle,
}

impl<D: ChromaDataSource> Vectorscope<D> {
    /// Create a vectorscope.
    pub fn new(
        _control: usize,
        data: Entity<D>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            data,
            focus_handle: cx.focus_handle(),
        }
    }

    /// The projected chroma points.
    pub fn points(&self, cx: &Context<Self>) -> Vec<(f32, f32)> {
        vectorscope_points(&self.data.read(cx).chroma_samples())
    }
}

impl<D: ChromaDataSource> Focusable for Vectorscope<D> {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl<D: ChromaDataSource> Render for Vectorscope<D> {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors().clone();
        let points = self.points(cx);
        canvas(
            move |_bounds, _window, _cx| (),
            move |bounds, (), window, _cx| {
                let width = f32::from(bounds.size.width);
                let height = f32::from(bounds.size.height);
                let center = point(bounds.left() + px(width / 2.0), bounds.top() + px(height / 2.0));
                let grid = Hsla::from(colors.border);
                // Graticule: crosshair + box.
                window.paint_quad(fill(
                    Bounds::new(
                        point(bounds.left(), center.y - px(0.5)),
                        size(bounds.size.width, px(1.0)),
                    ),
                    grid,
                ));
                window.paint_quad(fill(
                    Bounds::new(
                        point(center.x - px(0.5), bounds.top()),
                        size(px(1.0), bounds.size.height),
                    ),
                    grid,
                ));
                // Points: u -> x, v -> y (inverted).
                let point_color = Hsla::from(colors.selected);
                for &(u, v) in points.iter().take(4096) {
                    let x = center.x + px(u / 0.5 * width / 2.0);
                    let y = center.y - px(v / 0.5 * height / 2.0);
                    let dot = Bounds::new(point(x - px(1.0), y - px(1.0)), size(px(2.0), px(2.0)));
                    window.paint_quad(fill(dot, point_color));
                }
            },
        )
        .size_full()
    }
}
