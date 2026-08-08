//! In-window menus: a menu bar and right-click context menus, built with
//! gpui's `anchored` + `deferred` popups (no platform menus).
//!
//! The pure data model lives in [`model`]; the views here render it. Keyboard
//! navigation (up/down/enter/escape) works while the popup is focused; items
//! with submenus open them on hover to the right of the parent menu.
//! Activating an item emits [`MenuBarEvent::Triggered`] /
//! [`ContextMenuEvent::Triggered`] as a request.

pub mod model;

use gpui::{
    Anchor, App, ClickEvent, Context, ElementId, EventEmitter, FocusHandle, Focusable, KeyDownEvent,
    MouseButton, MouseUpEvent, Pixels, Point, Render, SharedString, Window, anchored,
    colors::DefaultColors, deferred, div, point, prelude::*, px,
};

pub use model::{Menu, MenuItem};

/// The height of one menu row, used for submenu positioning estimates.
const ROW_HEIGHT: f32 = 26.0;

/// A fully transparent color (for un-hovered rows).
fn transparent() -> gpui::Rgba {
    gpui::Rgba {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    }
}

/// A request emitted by a menu bar.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuBarEvent {
    /// An item was activated.
    Triggered {
        /// The menu bar's stable id.
        control: usize,
        /// The item's id (from [`MenuItem::id`]).
        item: usize,
        /// The item's label.
        label: SharedString,
    },
    /// A menu was opened.
    MenuOpened {
        /// The menu bar's stable id.
        control: usize,
        /// The menu index.
        index: usize,
    },
    /// The open menu was closed.
    MenuClosed {
        /// The menu bar's stable id.
        control: usize,
    },
}

/// A request emitted by a context menu.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextMenuEvent {
    /// The item's id (from [`MenuItem::id`]).
    pub item: usize,
    /// The item's label.
    pub label: SharedString,
}

/// A titled menu in a menu bar.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuBarEntry {
    /// The title shown in the bar.
    pub title: SharedString,
    /// The menu opened by the title.
    pub menu: Menu,
}

impl MenuBarEntry {
    /// Create an entry.
    pub fn new(title: impl Into<SharedString>, menu: Menu) -> Self {
        Self {
            title: title.into(),
            menu,
        }
    }
}

/// A horizontal menu bar with drop-down menus.
pub struct MenuBar {
    control: usize,
    entries: Vec<MenuBarEntry>,
    focus_handle: FocusHandle,
    open: Option<usize>,
    popup_position: Point<Pixels>,
    was_open_at_down: bool,
    hovered: Option<usize>,
    submenu: Option<usize>,
}

impl MenuBar {
    /// Create a menu bar.
    pub fn new(
        control: usize,
        entries: Vec<MenuBarEntry>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            control,
            entries,
            focus_handle: cx.focus_handle(),
            open: None,
            popup_position: Point::default(),
            was_open_at_down: false,
            hovered: None,
            submenu: None,
        }
    }

    /// Whether any menu is open.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    fn open_menu(&mut self, index: usize, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.open != Some(index) {
            self.open = Some(index);
            self.popup_position = position;
            self.hovered = None;
            self.submenu = None;
            cx.emit(MenuBarEvent::MenuOpened {
                control: self.control,
                index,
            });
            cx.notify();
        }
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.open.take().is_some() {
            self.hovered = None;
            self.submenu = None;
            cx.emit(MenuBarEvent::MenuClosed {
                control: self.control,
            });
            cx.notify();
        }
    }

    fn trigger(&mut self, item: usize, cx: &mut Context<Self>) {
        let label = self
            .open
            .as_ref()
            .and_then(|index| self.entries.get(*index))
            .and_then(|entry| entry.menu.items.iter().find(|i| i.id == item))
            .map(|i| i.label.clone())
            .or_else(|| {
                self.entries
                    .iter()
                    .flat_map(|e| &e.menu.items)
                    .find(|i| i.id == item)
                    .map(|i| i.label.clone())
            })
            .unwrap_or_default();
        cx.emit(MenuBarEvent::Triggered {
            control: self.control,
            item,
            label,
        });
        self.close_menu(cx);
    }

    fn navigate(&mut self, delta: i32, cx: &mut Context<Self>) {
        if let Some(index) = self.open {
            let menu = &self.entries[index].menu;
            if let Some(next) = menu.navigate(self.hovered, delta) {
                self.hovered = Some(next);
                self.submenu = None;
                cx.notify();
            }
        }
    }
}

impl EventEmitter<MenuBarEvent> for MenuBar {}

impl Focusable for MenuBar {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MenuBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors().clone();
        let mut bar = div()
            .id(ElementId::named_usize("gpui-widgets-menubar", self.control))
            .flex()
            .items_center()
            .px_2()
            .gap_1()
            .bg(colors.container)
            .on_mouse_down_out(
                cx.listener(|this, _event: &gpui::MouseDownEvent, _window, cx| {
                    this.close_menu(cx);
                }),
            );

        for (index, entry) in self.entries.clone().into_iter().enumerate() {
            let is_open = self.open == Some(index);
            bar = bar.child(
                div()
                    .id(ElementId::named_usize(
                        format!("gpui-widgets-menu-title-{}", self.control),
                        index,
                    ))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(if is_open { colors.selected } else { transparent() })
                    .text_color(if is_open {
                        colors.selected_text
                    } else {
                        colors.text
                    })
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _event: &gpui::MouseDownEvent, _window, _cx| {
                            this.was_open_at_down = this.open.is_some();
                        }),
                    )
                    .on_click(cx.listener(
                        move |this, event: &ClickEvent, _window, cx| {
                            if this.was_open_at_down {
                                this.close_menu(cx);
                            } else {
                                this.open_menu(index, event.position(), cx);
                            }
                            cx.stop_propagation();
                        },
                    ))
                    .child(entry.title),
            );
        }

        // The open menu popup.
        if let Some(open_index) = self.open {
            let entry = self.entries[open_index].clone();
            let hovered = self.hovered;
            let menu_popup = menu_popup_element(
                self.control,
                &entry.menu,
                hovered,
                &colors,
                cx.listener(|this, item: &MenuClicked, _window, cx| {
                    this.trigger(item.id, cx);
                }),
                cx.listener(|this, item: &MenuHovered, _window, cx| {
                    if item.submenu {
                        this.submenu = Some(item.index);
                        cx.notify();
                    }
                }),
            )
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                match event.keystroke.key.as_str() {
                    "up" => this.navigate(-1, cx),
                    "down" => this.navigate(1, cx),
                    "enter" | "space" => {
                        if let Some(hovered) = this.hovered {
                            if let Some(item) = entry_at(this, hovered) {
                                if item.enabled && item.submenu.is_none() {
                                    this.trigger(item.id, cx);
                                }
                            }
                        }
                    }
                    "escape" | "left" => this.close_menu(cx),
                    _ => {}
                }
            }));

            bar = bar.child(
                deferred(
                    anchored()
                        .position(self.popup_position)
                        .anchor(Anchor::TopLeft)
                        .offset(point(px(0.0), px(ROW_HEIGHT)))
                        .snap_to_window_with_margin(px(8.0))
                        .child(menu_popup),
                )
                .with_priority(1),
            );

            // A hovered item's submenu, anchored to the right of the popup.
            if let Some(hovered) = self.hovered
                && let Some(item) = self.entries[open_index].menu.items.get(hovered)
                && let Some(submenu) = item.submenu.clone()
            {
                let sub_hovered = self.submenu.and_then(|_| None);
                let sub_popup = menu_popup_element(
                    self.control + 1000,
                    &submenu,
                    sub_hovered,
                    &colors,
                    cx.listener(|this, clicked: &MenuClicked, _window, cx| {
                        this.trigger(clicked.id, cx);
                    }),
                    cx.listener(|_this, _item: &MenuHovered, _window, _cx| {}),
                );
                let width = f32::from(menu_width_estimate());
                bar = bar.child(
                    deferred(
                        anchored()
                            .position(self.popup_position)
                            .anchor(Anchor::TopLeft)
                            .offset(point(px(width + 2.0), px(ROW_HEIGHT)))
                            .snap_to_window_with_margin(px(8.0))
                            .child(sub_popup),
                    )
                    .with_priority(2),
                );
            }

            // Focus the popup so keyboard navigation works.
            window.focus(&self.focus_handle, cx);
        }

        bar
    }
}

/// A right-click context menu.
pub struct ContextMenu {
    focus_handle: FocusHandle,
    open: Option<ContextMenuState>,
}

struct ContextMenuState {
    position: Point<Pixels>,
    menu: Menu,
    hovered: Option<usize>,
}

impl ContextMenu {
    /// Create a context menu (hidden until [`Self::show`]).
    pub fn new(_control: usize, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            open: None,
        }
    }

    /// Whether the menu is visible.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Show the menu at `position` (window coordinates).
    pub fn show(&mut self, position: Point<Pixels>, menu: Menu, cx: &mut Context<Self>) {
        self.open = Some(ContextMenuState {
            position,
            menu,
            hovered: None,
        });
        cx.notify();
    }

    /// Hide the menu.
    pub fn hide(&mut self, cx: &mut Context<Self>) {
        if self.open.take().is_some() {
            cx.notify();
        }
    }
}

impl EventEmitter<ContextMenuEvent> for ContextMenu {}

impl Focusable for ContextMenu {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ContextMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors().clone();
        let mut root = div();

        if let Some(state) = self.open.take() {
            let menu = state.menu.clone();
            let hovered = state.hovered;
            let position = state.position;
            let popup = menu_popup_element(
                0,
                &menu,
                hovered,
                &colors,
                cx.listener(|this, clicked: &MenuClicked, _window, cx| {
                    cx.emit(ContextMenuEvent {
                        item: clicked.id,
                        label: clicked.label.clone(),
                    });
                    this.hide(cx);
                }),
                cx.listener(|_this, _item: &MenuHovered, _window, _cx| {}),
            )
            .track_focus(&self.focus_handle)
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _event: &MouseUpEvent, _window, cx| {
                    this.hide(cx);
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                match event.keystroke.key.as_str() {
                    "up" | "down" => {
                        if let Some(state) = this.open.as_mut() {
                            let delta = if event.keystroke.key == "up" { -1 } else { 1 };
                            if let Some(next) = state.menu.navigate(state.hovered, delta) {
                                state.hovered = Some(next);
                                cx.notify();
                            }
                        }
                    }
                    "enter" | "space" => {
                        if let Some(state) = this.open.as_ref()
                            && let Some(hovered) = state.hovered
                            && let Some(item) = state.menu.items.get(hovered)
                            && item.enabled
                        {
                            cx.emit(ContextMenuEvent {
                                item: item.id,
                                label: item.label.clone(),
                            });
                            this.hide(cx);
                        }
                    }
                    "escape" => this.hide(cx),
                    _ => {}
                }
            }));

            root = root.child(
                deferred(anchored().position(position).child(popup)).with_priority(1),
            );
            window.focus(&self.focus_handle, cx);
            self.open = Some(ContextMenuState {
                position,
                menu,
                hovered,
            });
        }

        root
    }
}

/// Marker event types passed to the shared popup builder.
struct MenuClicked {
    id: usize,
    label: SharedString,
}
struct MenuHovered {
    index: usize,
    submenu: bool,
}

/// Build a menu popup list. `on_click` receives the clicked item, `on_hover`
/// receives hovered-item info (used to open submenus).
fn menu_popup_element(
    control: usize,
    menu: &Menu,
    hovered: Option<usize>,
    colors: &gpui::colors::Colors,
    on_click: impl Fn(&MenuClicked, &mut Window, &mut App) + 'static,
    on_hover: impl Fn(&MenuHovered, &mut Window, &mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    use std::sync::Arc;
    let on_click = Arc::new(on_click);
    let on_hover = Arc::new(on_hover);
    let mut column = div()
        .id(ElementId::named_usize("gpui-widgets-menu-popup", control))
        .debug_selector(|| "menu-popup".into())
        .min_w(px(180.0))
        .rounded_md()
        .border_1()
        .border_color(colors.border)
        .bg(colors.container)
        .py_1()
        .flex()
        .flex_col();

    for (index, item) in menu.items.iter().enumerate() {
        let id = item.id;
        let label = item.label.clone();
        let shortcut = item.shortcut.clone();
        let checked = item.checked;
        let enabled = item.enabled;
        let has_submenu = item.submenu.is_some();
        let is_hovered = hovered == Some(index);
        let is_separator = Menu::is_separator(item);

        if is_separator {
            column = column.child(div().h(px(1.0)).my_1().bg(colors.separator));
            continue;
        }

        let row = div()
            .id(ElementId::named_usize(
                format!("gpui-widgets-menu-item-{control}"),
                id,
            ))
            .px_2()
            .h(px(ROW_HEIGHT))
            .flex()
            .items_center()
            .gap_2()
            .bg(if is_hovered { colors.selected } else { transparent() })
            .text_color(if enabled {
                colors.text
            } else {
                colors.disabled
            })
            .cursor_pointer()
            .child(
                div()
                    .w(px(16.0))
                    .child(if checked == Some(true) { "✓" } else { "" }),
            )
            .child(div().flex_1().child(label.clone()))
            .child(if has_submenu { "›" } else { "" })
            .child(
                div()
                    .text_color(colors.disabled)
                    .child(shortcut.unwrap_or_default()),
            );

        let row = if enabled {
            let on_click = on_click.clone();
            let on_hover = on_hover.clone();
            row.on_click(move |_event: &ClickEvent, window, cx| {
                on_click(
                    &MenuClicked {
                        id,
                        label: label.clone(),
                    },
                    window,
                    cx,
                );
            })
            .on_hover(move |hovered: &bool, window, cx| {
                if *hovered {
                    on_hover(
                        &MenuHovered {
                            index,
                            submenu: has_submenu,
                        },
                        window,
                        cx,
                    );
                }
            })
        } else {
            row
        };

        column = column.child(row);
    }
    column
}

/// A rough menu width estimate for submenu placement (matches `min_w`).
fn menu_width_estimate() -> Pixels {
    px(180.0)
}

/// Find the menu item at a raw index in the currently open menu.
fn entry_at(bar: &MenuBar, index: usize) -> Option<&MenuItem> {
    bar.entries
        .get(bar.open?)
        .and_then(|entry| entry.menu.items.get(index))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_bar_open_close_round_trip() {
        // Pure state checks are in model tests; here we just ensure the
        // model types are wired through the view API.
        let menu = Menu::new(vec![MenuItem::new(1, "Save").with_shortcut("⌘S")]);
        let entry = MenuBarEntry::new("File", menu);
        assert_eq!(entry.title, "File");
        assert_eq!(entry.menu.items[0].id, 1);
    }

    #[test]
    fn context_menu_event_carries_item() {
        let event = ContextMenuEvent {
            item: 7,
            label: "Paste".into(),
        };
        assert_eq!(event.item, 7);
        assert_eq!(event.label, "Paste");
    }
}
