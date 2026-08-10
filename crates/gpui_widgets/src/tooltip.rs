//! A small tooltip view for icon/toolbar buttons.
//!
//! gpui's [`Div::tooltip`](gpui::Div::tooltip) builder must return an
//! `AnyView`; this module supplies the standard Oak tooltip: a compact,
//! theme-colored label. Widgets attach it with
//! `el.tooltip(move |window, cx| tooltip_view(label.clone(), window, cx))`.

use gpui::{
    AnyView, App, Context, Render, SharedString, Window, colors::DefaultColors, div, prelude::*,
};

/// The tooltip view: a small rounded label in the theme's accent.
pub struct TooltipView {
    label: SharedString,
}

impl Render for TooltipView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors().clone();
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(colors.selected)
            .text_color(colors.selected_text)
            .text_xs()
            .child(self.label.clone())
    }
}

/// Builds a tooltip view for `label`, for use in a
/// [`Div::tooltip`](gpui::Div::tooltip) builder.
pub fn tooltip_view(label: SharedString, _window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|_cx| TooltipView { label }).into()
}
