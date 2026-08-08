//! A demo of the menu and dialog framework: a menu bar, a right-click
//! context menu, and the message/progress/file dialogs. Every action is
//! printed as a request event.

use gpui::{
    App, Bounds, Context, Entity, Focusable, MouseButton, MouseDownEvent, Render, Window,
    WindowBounds, WindowOptions, colors::DefaultColors, div, prelude::*, px, size,
};
use gpui_widgets::dialog::file_dialog::{FileDialogContent, file_dialog};
use gpui_widgets::dialog::message_box::{MessageBoxLevel, message_box};
use gpui_widgets::dialog::progress::{ProgressContent, progress_dialog};
use gpui_widgets::dialog::{Modal, ModalEvent};
use gpui_widgets::menu::{ContextMenu, ContextMenuEvent, Menu, MenuBar, MenuBarEntry, MenuBarEvent, MenuItem};

struct Example {
    menu_bar: Entity<MenuBar>,
    context_menu: Entity<ContextMenu>,
    message: Entity<Modal>,
    progress: Entity<Modal>,
    progress_content: Entity<ProgressContent>,
    file: Entity<Modal>,
    file_content: Entity<FileDialogContent>,
    show_message: bool,
    show_progress: bool,
    show_file: bool,
}

impl Example {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let menu_bar = cx.new(|cx| {
            MenuBar::new(
                1,
                vec![
                    MenuBarEntry::new(
                        "File",
                        Menu::new(vec![
                            MenuItem::new(10, "New…").with_shortcut("⌘N"),
                            MenuItem::new(11, "Open…").with_shortcut("⌘O"),
                            MenuItem::new(12, "Save").with_shortcut("⌘S").separated(),
                            MenuItem::new(13, "Export").disabled(),
                        ]),
                    ),
                    MenuBarEntry::new(
                        "Edit",
                        Menu::new(vec![
                            MenuItem::new(20, "Undo").with_shortcut("⌘Z"),
                            MenuItem::new(21, "Redo").with_shortcut("⇧⌘Z").separated(),
                            MenuItem::new(22, "Show Diagnostics").with_checked(false),
                        ]),
                    ),
                    MenuBarEntry::new(
                        "View",
                        Menu::new(vec![
                            MenuItem::new(30, "Toolbars"),
                            MenuItem::new(31, "Scopes"),
                            MenuItem::new(32, "Theme").with_submenu(Menu::new(vec![
                                MenuItem::new(33, "Olive Dark").with_checked(true),
                                MenuItem::new(34, "Olive Light"),
                            ])),
                        ]),
                    ),
                ],
                window,
                cx,
            )
        });
        cx.subscribe(
            &menu_bar,
            |_this: &mut Self, _m: Entity<MenuBar>, event: &MenuBarEvent, _cx| {
                println!("menu bar: {event:?}");
            },
        )
        .detach();

        let context_menu = cx.new(|cx| ContextMenu::new(2, window, cx));
        cx.subscribe(
            &context_menu,
            |_this: &mut Self, _m: Entity<ContextMenu>, event: &ContextMenuEvent, _cx| {
                println!("context menu: {event:?}");
            },
        )
        .detach();

        let message = message_box(
            3,
            MessageBoxLevel::Warning,
            "Unsaved changes",
            "Your project has unsaved changes. Export anyway?",
            window,
            cx,
        );
        cx.subscribe(
            &message,
            |this: &mut Self, _m: Entity<Modal>, event: &ModalEvent, _cx| {
                println!("message box: {event:?}");
                if matches!(event, ModalEvent::ButtonClicked { .. }) {
                    this.show_message = false;
                }
            },
        )
        .detach();

        let (progress, progress_content) = progress_dialog(4, "Exporting…", "Encoding video", window, cx);
        cx.subscribe(
            &progress,
            |this: &mut Self, _m: Entity<Modal>, event: &ModalEvent, _cx| {
                println!("progress: {event:?}");
                if matches!(event, ModalEvent::ButtonClicked { .. }) {
                    this.show_progress = false;
                }
            },
        )
        .detach();

        let (file, file_content) = file_dialog(5, "Open media…", window, cx);
        cx.subscribe(
            &file,
            |this: &mut Self, _m: Entity<Modal>, event: &ModalEvent, _cx| {
                if let ModalEvent::ButtonClicked { button: 0, .. } = event {
                    let path = this.file_content.read(_cx).path(_cx);
                    println!("open file: {path}");
                }
                println!("file dialog: {event:?}");
                if matches!(event, ModalEvent::ButtonClicked { .. }) {
                    this.show_file = false;
                }
            },
        )
        .detach();

        Self {
            menu_bar,
            context_menu,
            message,
            progress,
            progress_content,
            file,
            file_content,
            show_message: false,
            show_progress: false,
            show_file: false,
        }
    }
}

impl Render for Example {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors().clone();

        let mut root = div()
            .size_full()
            .bg(colors.background)
            .flex()
            .flex_col()
            .child(self.menu_bar.clone())
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_color(colors.text)
                    .id("example-main")
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                            this.context_menu.update(cx, |menu, cx| {
                                menu.show(
                                    event.position,
                                Menu::new(vec![
                                    MenuItem::new(40, "Cut").with_shortcut("⌘X"),
                                    MenuItem::new(41, "Copy").with_shortcut("⌘C"),
                                    MenuItem::new(42, "Paste").with_shortcut("⌘V"),
                                    MenuItem::new(43, "Delete").disabled().separated(),
                                    MenuItem::new(44, "Properties…"),
                                ]),
                                    cx,
                                );
                            });
                        }),
                    )
                    .child(div().child("Right-click for a context menu"))
                    .child(
                        div()
                            .id("example-btn-message")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(colors.selected)
                            .text_color(colors.selected_text)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _event: &gpui::ClickEvent, window, cx| {
                                this.show_message = true;
                                window.focus(&this.message.read(cx).focus_handle(cx), cx);
                            }))
                            .child("Message box"),
                    )
                    .child(
                        div()
                            .id("example-btn-progress")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(colors.selected)
                            .text_color(colors.selected_text)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _event: &gpui::ClickEvent, window, cx| {
                                this.show_progress = true;
                                this.progress_content
                                    .update(cx, |content, cx| content.set_progress(0.4, cx));
                                window.focus(&this.progress.read(cx).focus_handle(cx), cx);
                            }))
                            .child("Progress"),
                    )
                    .child(
                        div()
                            .id("example-btn-file")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(colors.selected)
                            .text_color(colors.selected_text)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _event: &gpui::ClickEvent, window, cx| {
                                this.show_file = true;
                                window.focus(&this.file.read(cx).focus_handle(cx), cx);
                            }))
                            .child("Open file…"),
                    ),
            )
            .child(self.context_menu.clone());

        if self.show_message {
            root = root.child(self.message.clone());
        }
        if self.show_progress {
            root = root.child(self.progress.clone());
        }
        if self.show_file {
            root = root.child(self.file.clone());
        }
        root
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        cx.init_colors();
        let bounds = Bounds::centered(None, size(px(640.0), px(480.0)), cx);
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
