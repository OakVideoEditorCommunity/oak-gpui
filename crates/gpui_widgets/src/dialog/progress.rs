//! A progress dialog: a cancelable dialog with a progress bar.
//!
//! The host updates the bar via [`ProgressContent::set_progress`]; the Cancel
//! button (index `1`) emits the modal's button event.

use gpui::{App, Context, Entity, Render, Window, colors::DefaultColors, div, prelude::*, px};

use super::{DialogButton, Modal, ModalOptions};

/// The content view of a progress dialog: a labeled progress bar.
pub struct ProgressContent {
    label: gpui::SharedString,
    /// Progress in `0..=1`.
    fraction: f32,
}

impl ProgressContent {
    /// Create a progress content view.
    pub fn new(label: impl Into<gpui::SharedString>, fraction: f32) -> Self {
        Self {
            label: label.into(),
            fraction: fraction.clamp(0.0, 1.0),
        }
    }

    /// Update the progress and repaint.
    pub fn set_progress(&mut self, fraction: f32, cx: &mut Context<Self>) {
        self.fraction = fraction.clamp(0.0, 1.0);
        cx.notify();
    }

    /// The current progress.
    pub fn fraction(&self) -> f32 {
        self.fraction
    }
}

impl Render for ProgressContent {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors().clone();
        let fraction = self.fraction;
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_color(colors.text).child(self.label.clone()))
            .child(
                div()
                    .h(px(10.0))
                    .rounded_full()
                    .bg(colors.background)
                    .border_1()
                    .border_color(colors.border)
                    .overflow_hidden()
                    .child(
                        div()
                            .h_full()
                            .w(px((fraction * 100.0).clamp(0.0, 100.0)))
                            .bg(colors.selected),
                    ),
            )
            .child(
                div()
                    .text_color(colors.disabled)
                    .child(format!("{:.0}%", fraction * 100.0)),
            )
    }
}

/// Build a progress dialog with a Cancel button (index `1`) and an implicit
/// primary "Run" button (index `0`). Returns the modal and its content so the
/// host can drive the bar.
pub fn progress_dialog(
    control: usize,
    title: impl Into<gpui::SharedString>,
    label: impl Into<gpui::SharedString>,
    window: &mut Window,
    cx: &mut App,
) -> (Entity<Modal>, Entity<ProgressContent>) {
    let content = cx.new(|_| ProgressContent::new(label, 0.0));
    let modal = cx.new(|cx| {
        Modal::new(
            control,
            ModalOptions::new(title, px(360.0))
                .with_button(DialogButton::primary("Run"))
                .with_button(DialogButton::cancel("Cancel")),
            window,
            cx,
        )
        .with_content(content.clone())
    });
    (modal, content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[test]
    fn progress_clamps_at_construction() {
        assert_eq!(ProgressContent::new("Encoding", 2.0).fraction(), 1.0);
        assert_eq!(ProgressContent::new("Encoding", -0.5).fraction(), 0.0);
    }

    #[gpui::test]
    async fn set_progress_clamps(cx: &mut TestAppContext) {
        cx.update(|app| {
            let content = app.new(|_| ProgressContent::new("Encoding", 0.0));
            content.update(app, |content, cx| {
                content.set_progress(1.5, cx);
                assert_eq!(content.fraction(), 1.0);
                content.set_progress(-1.0, cx);
                assert_eq!(content.fraction(), 0.0);
            });
        });
    }
}
