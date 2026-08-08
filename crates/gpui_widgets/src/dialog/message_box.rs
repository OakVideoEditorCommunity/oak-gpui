//! A message box: an informational dialog with an icon and a message.

use gpui::{App, Context, Entity, Render, Window, colors::DefaultColors, div, prelude::*, px};

use super::{DialogButton, Modal, ModalOptions};

/// The severity of a message box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageBoxLevel {
    /// Informational.
    Info,
    /// A warning.
    Warning,
    /// An error.
    Error,
}

impl MessageBoxLevel {
    /// The icon color for this level.
    fn color(self) -> gpui::Hsla {
        match self {
            MessageBoxLevel::Info => gpui::Hsla {
                h: 0.6,
                s: 0.8,
                l: 0.5,
                a: 1.0,
            },
            MessageBoxLevel::Warning => gpui::Hsla {
                h: 0.1,
                s: 0.9,
                l: 0.5,
                a: 1.0,
            },
            MessageBoxLevel::Error => gpui::Hsla {
                h: 0.0,
                s: 0.8,
                l: 0.5,
                a: 1.0,
            },
        }
    }

    /// The glyph shown next to the message.
    fn glyph(self) -> &'static str {
        match self {
            MessageBoxLevel::Info => "ℹ",
            MessageBoxLevel::Warning => "⚠",
            MessageBoxLevel::Error => "✖",
        }
    }
}

/// The content view of a message box.
struct MessageContent {
    level: MessageBoxLevel,
    message: gpui::SharedString,
}

impl Render for MessageContent {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors().clone();
        div()
            .flex()
            .items_start()
            .gap_3()
            .child(
                div()
                    .w(px(24.0))
                    .text_color(self.level.color())
                    .child(self.level.glyph()),
            )
            .child(div().flex_1().text_color(colors.text).child(self.message.clone()))
    }
}

/// Build a message box modal.
///
/// The host renders the returned [`Modal`] on top of its content and
/// subscribes to its [`ModalEvent`](super::ModalEvent)s (the OK button is
/// button index `0`).
pub fn message_box(
    control: usize,
    level: MessageBoxLevel,
    title: impl Into<gpui::SharedString>,
    message: impl Into<gpui::SharedString>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<Modal> {
    let content = cx.new(|_| MessageContent {
        level,
        message: message.into(),
    });
    let modal = cx.new(|cx| {
        Modal::new(
            control,
            ModalOptions::new(title, px(380.0)).with_button(DialogButton::primary("OK")),
            window,
            cx,
        )
        .with_content(content)
    });
    modal
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext, VisualTestContext, px, size};

    #[test]
    fn levels_have_distinct_glyphs() {
        assert_ne!(MessageBoxLevel::Info.glyph(), MessageBoxLevel::Error.glyph());
        assert_ne!(MessageBoxLevel::Info.color(), MessageBoxLevel::Warning.color());
    }

    #[gpui::test]
    async fn ok_button_emits_index_zero(cx: &mut TestAppContext) {
        struct Host {
            modal: Entity<Modal>,
            events: Vec<super::super::ModalEvent>,
        }
        impl Render for Host {
            fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
                div().size_full().child(self.modal.clone())
            }
        }

        cx.update(|cx| cx.init_colors());
        let window = cx.open_window(size(px(500.0), px(300.0)), |window, cx| {
            let modal = message_box(
                1,
                MessageBoxLevel::Error,
                "Render failed",
                "The export could not be completed.",
                window,
                cx,
            );
            let host = Host {
                modal,
                events: Vec::new(),
            };
            cx.subscribe(
                &host.modal,
                |host: &mut Host,
                 _m: Entity<Modal>,
                 event: &super::super::ModalEvent,
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
        // Click the OK button (index 0).
        let ok = cx.debug_bounds("dialog-button-0").expect("OK button rendered");
        cx.simulate_click(ok.center(), Modifiers::none());
        cx.run_until_parked();

        let routed = cx.read(|app| {
            host.read(app).events.iter().any(|e| {
                matches!(
                    e,
                    super::super::ModalEvent::ButtonClicked { button: 0, .. }
                )
            })
        });
        assert!(routed);
    }
}
