//! A file dialog: a modal with a path field and OK/Cancel buttons.
//!
//! This fork has no platform open/save panels yet (`prompt_for_paths` /
//! `prompt_for_new_path` do not exist in gpui), so the dialog is a native
//! gpui modal with a directly-editable path field. It emits
//! [`ModalEvent::ButtonClicked`] with the path available from
//! [`FileDialogContent::path`]; a host can back it with a real platform
//! picker later.

use gpui::{
    App, Context, Entity, Render, SharedString, Window, colors::DefaultColors, div, prelude::*,
    px,
};
use gpui_elements::editable_text::{EditableTextState, StringStorage, text_input};

use super::{DialogButton, Modal, ModalOptions};

/// The content view of a file dialog: a path text field.
pub struct FileDialogContent {
    editor: Entity<EditableTextState>,
}

impl FileDialogContent {
    /// The path currently entered.
    pub fn path(&self, app: &gpui::App) -> SharedString {
        self.editor.read(app).as_str().into()
    }

    /// Set the path shown in the field.
    pub fn set_path(&mut self, path: impl Into<SharedString>, cx: &mut Context<Self>) {
        let path = path.into();
        self.editor.update(cx, |editor, cx| {
            editor.emplace(path.as_ref(), cx);
        });
        cx.notify();
    }
}

impl Render for FileDialogContent {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors().clone();
        let weak = self.editor.downgrade();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_color(colors.text).child("Path"))
            .child(
                div()
                    .rounded_md()
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.background)
                    .px_2()
                    .py_1()
                    .child(text_input("gpui-widgets-file-path").state(weak).accepts_input(true)),
            )
    }
}

/// Build a file dialog. OK is button index `0`, Cancel is index `1`. The host
/// reads the chosen path from [`FileDialogContent::path`] when OK is clicked.
pub fn file_dialog(
    control: usize,
    title: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) -> (Entity<Modal>, Entity<FileDialogContent>) {
    let content = cx.new(|cx| {
        let editor = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
        FileDialogContent { editor }
    });
    let modal = cx.new(|cx| {
        Modal::new(
            control,
            ModalOptions::new(title, px(420.0))
                .with_button(DialogButton::primary("Open"))
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
    use gpui::{Entity, Render, TestAppContext, VisualTestContext, Window, div, px, size};

    #[gpui::test]
    async fn path_round_trips(cx: &mut TestAppContext) {
        cx.update(|app| {
            let content = app.new(|cx| {
                let editor = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
                FileDialogContent { editor }
            });
            content.update(app, |content, cx| {
                content.set_path("/tmp/movie.mov", cx);
                assert_eq!(content.path(cx), "/tmp/movie.mov");
            });
        });
    }

    #[gpui::test]
    async fn file_dialog_content_renders(cx: &mut TestAppContext) {
        struct Host {
            content: Entity<FileDialogContent>,
        }
        impl Render for Host {
            fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
                div().size_full().child(self.content.clone())
            }
        }

        cx.update(|cx| cx.init_colors());
        let window = cx.open_window(size(px(400.0), px(120.0)), |_window, cx| {
            let content = cx.new(|cx| {
                let editor = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
                FileDialogContent { editor }
            });
            content.update(cx, |content, cx| content.set_path("/tmp/movie.mov", cx));
            Host { content }
        });
        cx.run_until_parked();
        let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
        cx.update(|window, cx| {
            window.draw(cx).clear();
        });
    }
}
