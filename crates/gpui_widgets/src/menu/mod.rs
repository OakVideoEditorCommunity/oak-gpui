//! In-window menus: a menu bar and right-click context menus, built with
//! gpui's `anchored` + `deferred` popups (no platform menus).
//!
//! The pure data model lives in [`model`]; the views here render it. Keyboard
//! navigation (up/down/enter/escape) works while the popup is focused; items
//! with submenus open them on hover to the right of the parent menu. With a
//! menu open, hovering another top-level title switches to that menu (menu
//! scrubbing), like a native menu bar.
//! Activating an item emits [`MenuBarEvent::Triggered`] /
//! [`ContextMenuEvent::Triggered`] as a request.

pub mod model;

use gpui::{
	Anchor, App, ClickEvent, Context, ElementId, EventEmitter, FocusHandle, Focusable,
	KeyDownEvent, MouseButton, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, SharedString,
	Window, anchored, colors::DefaultColors, deferred, div, point, prelude::*, px,
};

pub use model::{Menu, MenuItem};

/// The height of one menu row, used for submenu positioning estimates.
const ROW_HEIGHT: f32 = 22.0;

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

	/// Sets the checked state of the menu item with `id` across all entries
	/// (searching submenus recursively), so a host can toggle a checkmark at
	/// runtime without rebuilding the [`MenuBar`].
	///
	/// The checkmark appears on the next repaint (the renderer reads
	/// `checked` per frame); pair with a `cx.notify()` after the call.
	/// Returns whether an item with that id was found.
	pub fn set_item_checked(&mut self, id: usize, checked: bool) -> bool {
		let mut found = false;
		for entry in &mut self.entries {
			found |= entry.menu.set_item_checked(id, checked);
		}
		found
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
			.or_else(|| self.find_label(item))
			.unwrap_or_default();
		cx.emit(MenuBarEvent::Triggered {
			control: self.control,
			item,
			label,
		});
		self.close_menu(cx);
	}

	/// The label of an item with `id` anywhere in the open menu, including
	/// nested submenus (the fallback when the item is not a top-level row).
	fn find_label(&self, item: usize) -> Option<SharedString> {
		let index = self.open?;
		fn search(menu: &Menu, item: usize) -> Option<SharedString> {
			menu.items.iter().find_map(|i| {
				if i.id == item {
					Some(i.label.clone())
				} else {
					i.submenu.as_deref().and_then(|sub| search(sub, item))
				}
			})
		}
		search(&self.entries[index].menu, item)
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
			.py_0p5()
			.gap_1()
			.bg(colors.container);

		for (index, entry) in self.entries.clone().into_iter().enumerate() {
			let is_open = self.open == Some(index);
			bar = bar.child(
				div()
					.id(ElementId::named_usize(
						format!("gpui-widgets-menu-title-{}", self.control),
						index,
					))
					.debug_selector(move || format!("menu-title-{index}").into())
					.px_2()
					.py_0p5()
					.rounded_md()
					.bg(if is_open {
						colors.selected
					} else {
						transparent()
					})
					.text_color(if is_open {
						colors.selected_text
					} else {
						colors.text
					})
					.text_xs()
					.cursor_pointer()
					.on_mouse_down(
						MouseButton::Left,
						cx.listener(|this, _event: &gpui::MouseDownEvent, _window, _cx| {
							this.was_open_at_down = this.open.is_some();
						}),
					)
					.on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _window, cx| {
						// Menu scrubbing: with a menu open, moving over another
						// title switches the open menu to it (re-anchoring the
						// popup under the pointer). Nothing happens while no
						// menu is open, so the titles only open on click.
						if this.open.is_some() {
							this.open_menu(index, event.position, cx);
						}
					}))
					.on_click(cx.listener(move |this, event: &ClickEvent, _window, cx| {
						if this.was_open_at_down {
							this.close_menu(cx);
						} else {
							this.open_menu(index, event.position(), cx);
						}
						cx.stop_propagation();
					}))
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
				"menu-popup",
				&colors,
				cx.listener(|this, item: &MenuClicked, _window, cx| {
					this.trigger(item.id, cx);
				}),
				cx.listener(|this, item: &MenuHovered, _window, cx| {
					// Mouse hover drives both the row highlight and the
					// submenu: remember the hovered row so the render pass
					// can open the nested menu next to it.
					this.hovered = Some(item.index);
					this.submenu = if item.submenu { Some(item.index) } else { None };
					cx.notify();
				}),
			)
			.track_focus(&self.focus_handle)
			.on_mouse_up_out(
				MouseButton::Left,
				cx.listener(|this, _event: &MouseUpEvent, _window, cx| {
					this.close_menu(cx);
				}),
			)
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
					"menu-submenu-popup",
					&colors,
					cx.listener(|this, clicked: &MenuClicked, _window, cx| {
						this.trigger(clicked.id, cx);
					}),
					cx.listener(|_this, _item: &MenuHovered, _window, _cx| {}),
				);
				let width = f32::from(menu_width(&entry.menu));
				// Align the submenu with the hovered row (one row height per
				// item) so it opens next to the item, not the first row.
				let top = ROW_HEIGHT * (hovered as f32 + 1.0);
				bar = bar.child(
					deferred(
						anchored()
							.position(self.popup_position)
							.anchor(Anchor::TopLeft)
							.offset(point(px(width + 2.0), px(top)))
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
	/// The row whose submenu is open (same one-level model as the menu
	/// bar: a hovered parent opens its submenu to the right).
	submenu: Option<usize>,
	/// The hovered row inside the open submenu (row highlight).
	submenu_hovered: Option<usize>,
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
			submenu: None,
			submenu_hovered: None,
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
			let submenu = state.submenu;
			let submenu_hovered = state.submenu_hovered;
			let position = state.position;
			let popup = menu_popup_element(
				0,
				&menu,
				hovered,
				"menu-popup",
				&colors,
				cx.listener(|this, clicked: &MenuClicked, _window, cx| {
					cx.emit(ContextMenuEvent {
						item: clicked.id,
						label: clicked.label.clone(),
					});
					this.hide(cx);
				}),
				cx.listener(|this, item: &MenuHovered, _window, cx| {
					// Mouse hover drives the row highlight and opens the
					// hovered parent's submenu (the menu-bar behavior).
					if let Some(state) = this.open.as_mut() {
						state.hovered = Some(item.index);
						state.submenu = if item.submenu { Some(item.index) } else { None };
						state.submenu_hovered = None;
						cx.notify();
					}
				}),
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
								state.submenu = state
									.menu
									.items
									.get(next)
									.and_then(|item| item.submenu.as_ref().map(|_| next));
								cx.notify();
							}
						}
					}
					"right" => {
						// Open the highlighted parent's submenu (also the
						// behavior of Enter on a parent row).
						if let Some(state) = this.open.as_mut()
							&& let Some(hovered) = state.hovered
							&& let Some(item) = state.menu.items.get(hovered)
							&& item.submenu.is_some()
						{
							state.submenu = Some(hovered);
							state.submenu_hovered = None;
							cx.notify();
						}
					}
					"left" => {
						if let Some(state) = this.open.as_mut()
							&& state.submenu.take().is_some()
						{
							cx.notify();
						}
					}
					"enter" | "space" => {
						if let Some(state) = this.open.as_mut()
							&& let Some(hovered) = state.hovered
							&& let Some(item) = state.menu.items.get(hovered)
							&& item.enabled
						{
							if item.submenu.is_some() {
								state.submenu = Some(hovered);
								state.submenu_hovered = None;
								cx.notify();
							} else {
								let event = ContextMenuEvent {
									item: item.id,
									label: item.label.clone(),
								};
								cx.emit(event);
								this.hide(cx);
							}
						}
					}
					"escape" => this.hide(cx),
					_ => {}
				}
			}));

			root =
				root.child(deferred(anchored().position(position).child(popup)).with_priority(1));

			// A hovered/opened parent's submenu, anchored to the right of
			// the popup (mirrors the menu bar's submenu placement).
			if let Some(hovered) = hovered
				&& submenu == Some(hovered)
				&& let Some(item) = menu.items.get(hovered)
				&& let Some(submenu_menu) = item.submenu.clone()
			{
				let sub_popup = menu_popup_element(
					1000,
					&submenu_menu,
					submenu_hovered,
					"menu-submenu-popup",
					&colors,
					cx.listener(|this, clicked: &MenuClicked, _window, cx| {
						cx.emit(ContextMenuEvent {
							item: clicked.id,
							label: clicked.label.clone(),
						});
						this.hide(cx);
					}),
					cx.listener(|this, item: &MenuHovered, _window, cx| {
						if let Some(state) = this.open.as_mut() {
							state.submenu_hovered = Some(item.index);
							cx.notify();
						}
					}),
				);
				let width = f32::from(menu_width(&menu));
				// Align the submenu with the parent's row so it opens next
				// to the item, not the first row.
				let top = ROW_HEIGHT * (hovered as f32 + 1.0);
				root = root.child(
					deferred(
						anchored()
							.position(position)
							.offset(point(px(width + 2.0), px(top)))
							.snap_to_window_with_margin(px(8.0))
							.child(sub_popup),
					)
					.with_priority(2),
				);
			}

			window.focus(&self.focus_handle, cx);
			self.open = Some(ContextMenuState {
				position,
				menu,
				hovered,
				submenu,
				submenu_hovered,
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
/// receives hovered-item info (used to open submenus). `debug_key` is the
/// test selector registered for the popup's bounds.
fn menu_popup_element(
	control: usize,
	menu: &Menu,
	hovered: Option<usize>,
	debug_key: &'static str,
	colors: &gpui::colors::Colors,
	on_click: impl Fn(&MenuClicked, &mut Window, &mut App) + 'static,
	on_hover: impl Fn(&MenuHovered, &mut Window, &mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
	use std::sync::Arc;
	let on_click = Arc::new(on_click);
	let on_hover = Arc::new(on_hover);
	let mut column = div()
		.id(ElementId::named_usize("gpui-widgets-menu-popup", control))
		.debug_selector(move || debug_key.into())
		.w(menu_width(menu))
		.rounded_md()
		.border_1()
		.border_color(colors.border)
		.bg(colors.container)
		.py_1()
		.text_xs()
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
			.bg(if is_hovered {
				colors.selected
			} else {
				transparent()
			})
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

/// A menu popup's content-aware width: the longest label (CJK glyphs
/// count double) plus the checkmark, submenu-arrow and shortcut columns,
/// clamped to a sane range. The popup sizes itself with this and the
/// submenu x-offset reads the same value, so the two never disagree.
fn menu_width(menu: &Menu) -> Pixels {
	let mut max_units = 0.0f32;
	for item in &menu.items {
		if Menu::is_separator(item) {
			continue;
		}
		let label: f32 = item
			.label
			.chars()
			.map(|c| if c.is_ascii() { 1.0 } else { 2.0 })
			.sum();
		let shortcut = item
			.shortcut
			.as_ref()
			.map(|s| s.chars().count() as f32 + 2.0)
			.unwrap_or(0.0);
		max_units = max_units.max(label + shortcut);
	}
	(px(16.0 + 48.0) + px(max_units * 7.0)).clamp(px(96.0), px(360.0))
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
	use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, point, px, size};

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

	// --- interaction tests for the menu views ---

	fn demo_entries() -> Vec<MenuBarEntry> {
		vec![MenuBarEntry::new(
			"File",
			Menu::new(vec![
				MenuItem::new(10, "Open…").with_shortcut("⌘O"),
				MenuItem::new(11, "Save").with_shortcut("⌘S"),
				MenuItem::new(12, "Quit").separated(),
			]),
		)]
	}

	/// An entry whose first item carries a nested submenu (like the app's
	/// 视图 → 语言 / 主题).
	fn demo_entries_with_submenu() -> Vec<MenuBarEntry> {
		let sub = Menu::new(vec![
			MenuItem::new(21, "简体中文"),
			MenuItem::new(22, "English"),
		]);
		vec![MenuBarEntry::new(
			"View",
			Menu::new(vec![
				MenuItem::new(20, "Language").with_submenu(sub),
				MenuItem::new(23, "Preferences…"),
			]),
		)]
	}

	/// Two top-level menus ("File" then "View"), for scrubbing between titles.
	fn demo_entries_two() -> Vec<MenuBarEntry> {
		vec![
			MenuBarEntry::new(
				"File",
				Menu::new(vec![
					MenuItem::new(10, "Open…").with_shortcut("⌘O"),
					MenuItem::new(11, "Save").with_shortcut("⌘S"),
					MenuItem::new(12, "Quit").separated(),
				]),
			),
			MenuBarEntry::new(
				"View",
				Menu::new(vec![
					MenuItem::new(20, "Language"),
					MenuItem::new(23, "Preferences…"),
				]),
			),
		]
	}

	struct Host {
		menu_bar: Entity<MenuBar>,
		events: Vec<MenuBarEvent>,
	}
	impl Render for Host {
		fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
			div().size_full().child(self.menu_bar.clone())
		}
	}

	fn make_bar(cx: &mut TestAppContext) -> (&'static mut VisualTestContext, Entity<Host>) {
		make_bar_with(cx, demo_entries())
	}

	fn make_bar_with(
		cx: &mut TestAppContext,
		entries: Vec<MenuBarEntry>,
	) -> (&'static mut VisualTestContext, Entity<Host>) {
		cx.update(|cx| cx.init_colors());
		let window = cx.open_window(size(px(400.0), px(120.0)), |window, cx| {
			let menu_bar = cx.new(|cx| MenuBar::new(1, entries, window, cx));
			let host = Host {
				menu_bar,
				events: Vec::new(),
			};
			cx.subscribe(
				&host.menu_bar,
				|host: &mut Host,
				 _m: Entity<MenuBar>,
				 event: &MenuBarEvent,
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
		(cx, host)
	}

	#[gpui::test]
	async fn clicking_a_menu_title_opens_the_popup(cx: &mut TestAppContext) {
		let (cx, _host) = make_bar(cx);
		// Click the "File" title (top-left of the bar).
		cx.simulate_click(point(px(20.0), px(10.0)), Modifiers::none());
		cx.run_until_parked();
		cx.update(|window, cx| {
			window.draw(cx).clear();
		});

		let popup = cx.debug_bounds("menu-popup").expect("menu popup rendered");
		assert!(popup.size.height > px(60.0), "popup should list the items");
	}

	#[gpui::test]
	async fn clicking_a_menu_item_emits_triggered(cx: &mut TestAppContext) {
		let (cx, host) = make_bar(cx);
		cx.simulate_click(point(px(20.0), px(10.0)), Modifiers::none());
		cx.run_until_parked();
		cx.update(|window, cx| {
			window.draw(cx).clear();
		});

		let popup = cx.debug_bounds("menu-popup").expect("menu popup rendered");
		// The first item row is the first ~26px of the popup.
		cx.simulate_click(
			point(popup.left() + px(40.0), popup.top() + px(16.0)),
			Modifiers::none(),
		);
		cx.run_until_parked();

		let triggered = cx.read(|app| {
			host.read(app)
				.events
				.iter()
				.any(|e| matches!(e, MenuBarEvent::Triggered { item: 10, .. }))
		});
		assert!(triggered, "expected Triggered for the first item");
	}

	#[gpui::test]
	async fn keyboard_navigation_triggers_the_hovered_item(cx: &mut TestAppContext) {
		let (cx, host) = make_bar(cx);
		cx.simulate_click(point(px(20.0), px(10.0)), Modifiers::none());
		cx.run_until_parked();
		cx.update(|window, cx| {
			window.draw(cx).clear();
		});

		// Down from no selection lands on the first item; the second down
		// moves to Save. Enter triggers it.
		cx.simulate_keystrokes("down");
		cx.run_until_parked();
		cx.simulate_keystrokes("down");
		cx.run_until_parked();
		cx.simulate_keystrokes("enter");
		cx.run_until_parked();

		let triggered = cx.read(|app| {
			host.read(app)
				.events
				.iter()
				.any(|e| matches!(e, MenuBarEvent::Triggered { item: 11, .. }))
		});
		assert!(
			triggered,
			"expected Triggered for the second item via keyboard"
		);
	}

	#[gpui::test]
	async fn escape_closes_the_menu(cx: &mut TestAppContext) {
		let (cx, host) = make_bar(cx);
		cx.simulate_click(point(px(20.0), px(10.0)), Modifiers::none());
		cx.run_until_parked();
		cx.update(|window, cx| {
			window.draw(cx).clear();
		});
		assert!(cx.debug_bounds("menu-popup").is_some());

		cx.simulate_keystrokes("escape");
		cx.run_until_parked();
		cx.update(|window, cx| {
			window.draw(cx).clear();
		});

		assert!(
			cx.debug_bounds("menu-popup").is_none(),
			"menu should close on escape"
		);
		let closed = cx.read(|app| {
			host.read(app)
				.events
				.iter()
				.any(|e| matches!(e, MenuBarEvent::MenuClosed { .. }))
		});
		assert!(closed);
	}

	/// Menu scrubbing: while a menu is open, moving over another top-level
	/// title switches the open menu to it (and re-anchors the popup under that
	/// title). With nothing open, hovering does not open a menu.
	#[gpui::test]
	async fn hovering_another_title_switches_the_open_menu(cx: &mut TestAppContext) {
		let (cx, host) = make_bar_with(cx, demo_entries_two());
		// Open the first menu ("File").
		cx.simulate_click(point(px(20.0), px(10.0)), Modifiers::none());
		cx.run_until_parked();
		cx.update(|window, cx| {
			window.draw(cx).clear();
		});
		let popup = cx.debug_bounds("menu-popup").expect("first menu open");

		// Move over the second title ("View"): the open menu must switch to it.
		let second_title = cx
			.debug_bounds("menu-title-1")
			.expect("second title rendered");
		cx.simulate_mouse_move(second_title.center(), None, Modifiers::none());
		cx.run_until_parked();
		cx.update(|window, cx| {
			window.draw(cx).clear();
		});

		let switched = cx.read(|app| {
			host.read(app)
				.events
				.iter()
				.any(|e| matches!(e, MenuBarEvent::MenuOpened { index: 1, .. }))
		});
		assert!(
			switched,
			"hovering the second title should switch the open menu"
		);
		let moved = cx.debug_bounds("menu-popup").expect("popup stays open");
		assert!(
			moved.left() > popup.left() + px(10.0),
			"the popup follows the hovered title"
		);

		// Hovering back over the first title switches back.
		let first_title = cx
			.debug_bounds("menu-title-0")
			.expect("first title rendered");
		cx.simulate_mouse_move(first_title.center(), None, Modifiers::none());
		cx.run_until_parked();
		cx.update(|window, cx| {
			window.draw(cx).clear();
		});
		let switched_back = cx.read(|app| {
			host.read(app)
				.events
				.iter()
				.filter(|e| matches!(e, MenuBarEvent::MenuOpened { index: 0, .. }))
				.count()
				>= 2
		});
		assert!(switched_back, "hovering the first title switches back");
	}

	#[gpui::test]
	async fn runtime_set_item_checked_flips_the_checkmark(cx: &mut TestAppContext) {
		let (cx, host) = make_bar(cx);
		// Toggle item 11 ("Save") at runtime, by id.
		let changed = cx.update(|_window, app| {
			let bar = host.read(app).menu_bar.clone();
			bar.update(app, |bar, _cx| bar.set_item_checked(11, true))
		});
		assert!(changed, "item 11 exists and should be updated");
		let checked = cx.update(|_window, app| {
			host.read(app).menu_bar.read(app).entries[0].menu.items[1].checked
		});
		assert_eq!(checked, Some(true));

		// Unknown ids are reported as not found and change nothing.
		let changed = cx.update(|_window, app| {
			let bar = host.read(app).menu_bar.clone();
			bar.update(app, |bar, _cx| bar.set_item_checked(12345, true))
		});
		assert!(!changed);
		let checked = cx.update(|_window, app| {
			host.read(app).menu_bar.read(app).entries[0].menu.items[0].checked
		});
		assert_eq!(checked, None);
	}

	/// Hovering a menu item that carries a nested menu must open the second
	/// level to the right of the popup (the app's 视图 → 语言 / 主题 flow).
	#[gpui::test]
	async fn hovering_a_submenu_item_opens_the_submenu(cx: &mut TestAppContext) {
		let (cx, host) = make_bar_with(cx, demo_entries_with_submenu());
		// Click the "View" title to open the menu.
		cx.simulate_click(point(px(20.0), px(10.0)), Modifiers::none());
		cx.run_until_parked();
		cx.update(|window, cx| {
			window.draw(cx).clear();
		});

		let popup = cx.debug_bounds("menu-popup").expect("menu popup rendered");
		// Hover the first row ("Language", which has the submenu).
		cx.simulate_mouse_move(
			point(popup.left() + px(40.0), popup.top() + px(13.0)),
			None,
			Modifiers::none(),
		);
		cx.run_until_parked();
		cx.update(|window, cx| {
			window.draw(cx).clear();
		});

		let submenu = cx
			.debug_bounds("menu-submenu-popup")
			.expect("submenu popup opens next to the hovered item");
		assert!(
			submenu.left() > popup.left(),
			"the submenu opens to the right of the parent popup"
		);

		// Clicking a submenu row triggers it (and closes the menus).
		cx.simulate_click(
			point(submenu.left() + px(40.0), submenu.top() + px(13.0)),
			Modifiers::none(),
		);
		cx.run_until_parked();
		let triggered = cx.read(|app| {
			host.read(app)
				.events
				.iter()
				.any(|e| matches!(e, MenuBarEvent::Triggered { item: 21, .. }))
		});
		assert!(triggered, "expected Triggered for the submenu item");
		assert!(
			cx.debug_bounds("menu-popup").is_none(),
			"menu closed after triggering"
		);
	}
}
