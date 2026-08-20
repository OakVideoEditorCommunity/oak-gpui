//! The tab strip rendered above each `Tabs` group in a
//! [`DockArea`](crate::dock::DockArea).
//!
//! Internal component — not part of the public API. One `TabBar` exists per
//! [`DockNode::Tabs`](crate::dock::DockNode::Tabs) node; it renders one tab
//! per panel (via [`DockPanel::tab_content`](crate::dock::DockPanel::tab_content)),
//! tracks the active tab, hosts close buttons, and is the drag source for
//! both tab reordering and dock drags.

use crate::colors::DefaultColors;
use crate::{
	App, AppContext, ClickEvent, Context, DragMoveEvent, ElementId, EventEmitter,
	InteractiveElement, IntoElement, ParentElement, Pixels, Point, Render, ScrollDelta,
	ScrollWheelEvent, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};

use super::PanelId;

/// Events emitted by a [`TabBar`] toward its owning [`DockArea`].
#[derive(Clone, Debug)]
pub(crate) enum TabBarEvent {
	/// The tab order or the active tab changed in place.
	Reordered {
		/// The new tab order (mirrors the owning `Tabs` node's `panels`).
		tabs: Vec<PanelId>,
		/// Index of the active tab in `tabs`.
		active: usize,
	},
	/// The user clicked the close button of a tab.
	CloseRequested(PanelId),
	/// A tab was dragged out of the strip; the dock area takes over the drag.
	DockDragStarted {
		/// The panel being dragged.
		panel: PanelId,
		/// Pointer position in window coordinates.
		position: Point<Pixels>,
	},
}

/// A single tab's computed geometry within the strip, cached during render
/// for hit-testing (close button, reorder, drag start).
pub(crate) struct TabGeometry {
	/// The panel this tab shows.
	#[allow(dead_code)] // retained for future hit-testing of individual tabs
	pub panel: PanelId,
	/// Left edge of the tab relative to the strip.
	pub x: Pixels,
	/// Width of the tab.
	pub width: Pixels,
}

/// The tab strip for one `Tabs` node.
///
/// # Responsibilities
///
/// - Render tabs in tree order, highlighting the active one and dimming
///   inactive ones.
/// - Show a close button on tabs whose panel reports
///   [`closable`](crate::dock::DockPanel::closable); clicking it emits
///   [`TabBarEvent::CloseRequested`], which the dock area routes through its
///   close flow (never removes the panel directly).
/// - Reorder tabs by dragging within the strip: dropping a tab between two
///   others emits [`TabBarEvent::Reordered`] and the dock area rewrites
///   [`DockNode::Tabs::panels`](crate::dock::DockNode::Tabs::panels) in place.
/// - Escalate to a dock drag: once a dragged tab leaves the strip's bounds,
///   emit [`TabBarEvent::DockDragStarted`] so the
///   [`DockArea`](crate::dock::DockArea)'s drag-to-dock plumbing takes over.
/// - Overflow: when tabs exceed the available width, the strip scrolls
///   horizontally (wheel and drag) instead of shrinking tabs below a minimum
///   width; the active tab is scrolled into view when activated.
///
/// # Invariants
///
/// `tabs` always mirrors the owning node's `panels` order exactly and
/// `active < tabs.len()`; the dock area re-creates or syncs the strip
/// whenever the tree changes rather than the strip mutating the tree itself.
pub(crate) struct TabBar {
	/// Panels in tab order, mirroring the owning `Tabs` node.
	tabs: Vec<PanelId>,
	/// Index of the active tab.
	active: usize,
	/// Horizontal scroll offset for overflowed strips.
	scroll_offset: Pixels,
	/// Per-tab geometry from the last frame, for hit-testing.
	geometry: Vec<TabGeometry>,
	/// Tab titles from the last sync, rendered as the tab labels.
	titles: Vec<SharedString>,
	/// Per-tab closability from the last sync.
	closable: Vec<bool>,
}

impl TabBar {
	/// Minimum width a tab is allowed to shrink to before the strip starts
	/// scrolling instead.
	pub(crate) const MIN_TAB_WIDTH: Pixels = Pixels(64.0);

	/// Estimated rendered width of a tab for its title: tabs size to their
	/// content (the design shows full `<面板>·<名称>` labels), so drag
	/// hit-testing estimates each tab's width from the title — CJK glyphs are
	/// full-width, ASCII roughly half — plus the horizontal padding and, for
	/// closable tabs, the right-aligned close button. Only the cached drag
	/// geometry uses this; the layout itself measures the text.
	fn estimated_width(title: &str, closable: bool) -> Pixels {
		let units: f32 = title
			.chars()
			.map(|ch| if ch.is_ascii() { 0.55 } else { 1.0 })
			.sum();
		// The tab's px-2 horizontal padding plus, for closable tabs, the ✕
		// button and its px-0.5 padding.
		let chrome = 20.0 + if closable { 16.0 } else { 0.0 };
		Pixels((units * 13.0 + chrome).max(Self::MIN_TAB_WIDTH.0))
	}

	/// Creates a strip for the given tabs; `active` is clamped into range.
	pub(crate) fn new(tabs: Vec<PanelId>, active: usize) -> Self {
		Self {
			active: active.min(tabs.len().saturating_sub(1)),
			tabs,
			scroll_offset: Pixels(0.0),
			geometry: Vec::new(),
			titles: Vec::new(),
			closable: Vec::new(),
		}
	}

	/// Syncs the strip with the owning node after a tree edit, preserving
	/// scroll position where possible.
	///
	/// Called from the dock area during render, so it must not notify.
	pub(crate) fn sync(
		&mut self,
		tabs: &[PanelId],
		active: usize,
		titles: &[SharedString],
		closable: &[bool],
		_cx: &mut Context<Self>,
	) {
		self.tabs = tabs.to_vec();
		self.titles = titles.to_vec();
		self.closable = closable.to_vec();
		self.active = active.min(self.tabs.len().saturating_sub(1));
	}

	/// Returns the index of the tab containing `point` (strip-relative
	/// coordinates), using the cached geometry.
	pub(crate) fn tab_index_at(&self, point: Point<Pixels>) -> Option<usize> {
		let count = self.geometry.partition_point(|tab| tab.x.0 <= point.x.0);
		if count == 0 {
			return None;
		}
		// `count - 1` is the last tab whose left edge is left of the point;
		// tabs are contiguous, so that tab contains the point.
		Some(count - 1)
	}

	/// Reorders the tab at `from` to position `to`, keeping the active tab
	/// pointing at the same panel, and emits [`TabBarEvent::Reordered`] so
	/// the dock area can rewrite the owning node.
	///
	/// No-op if either index is out of range or `from == to`.
	pub(crate) fn move_tab(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
		if from >= self.tabs.len() || to >= self.tabs.len() || from == to {
			return;
		}
		let active_panel = self.tabs.get(self.active).copied();
		let panel = self.tabs.remove(from);
		let insert_at = if to > from { to - 1 } else { to };
		self.tabs.insert(insert_at, panel);
		self.active = active_panel
			.and_then(|p| self.tabs.iter().position(|tab| *tab == p))
			.unwrap_or(0);
		cx.emit(TabBarEvent::Reordered {
			tabs: self.tabs.clone(),
			active: self.active,
		});
		cx.notify();
	}

	/// Makes the tab at `index` active, scrolling it into view first.
	fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
		if index >= self.tabs.len() {
			return;
		}
		self.active = index;
		self.scroll_tab_into_view(index, window, cx);
		cx.emit(TabBarEvent::Reordered {
			tabs: self.tabs.clone(),
			active: self.active,
		});
		cx.notify();
	}

	/// Scrolls the strip so the tab at `index` is fully visible.
	///
	/// The strip does not know the exact width of the visible viewport, so
	/// this approximates it with the window width — good enough to bring an
	/// overflowing tab back into view.
	fn scroll_tab_into_view(&mut self, index: usize, window: &mut Window, _cx: &mut Context<Self>) {
		let Some(geometry) = self.geometry.get(index) else {
			return;
		};
		let viewport = window.viewport_size().width.0;
		let left = geometry.x.0;
		let right = geometry.x.0 + geometry.width.0;
		let scrolled = self.scroll_offset.0;
		if left < scrolled {
			self.scroll_offset = Pixels(left.max(0.0));
		} else if right > scrolled + viewport {
			self.scroll_offset = Pixels((right - viewport).max(0.0));
		}
	}
}

impl EventEmitter<TabBarEvent> for TabBar {}

impl Render for TabBar {
	fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
		let colors = cx.default_colors().clone();

		// Recompute per-tab geometry from the current order and scroll offset.
		// Tabs are content-sized, so accumulate the estimated widths.
		{
			let mut x = -self.scroll_offset.0;
			self.geometry = self
				.tabs
				.iter()
				.enumerate()
				.map(|(index, &panel)| {
					let width = Self::estimated_width(
						self.titles.get(index).map(|t| t.as_ref()).unwrap_or(""),
						self.closable.get(index).copied().unwrap_or(false),
					);
					let tab = TabGeometry { panel, x: Pixels(x), width };
					x += width.0;
					tab
				})
				.collect();
		}

		let mut root = div()
			.flex()
			.flex_row()
			.items_center()
			.h(px(26.0))
			.w_full()
			.overflow_hidden()
			.on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _window, cx| {
				let delta = match event.delta {
					ScrollDelta::Pixels(delta) => delta.x.0,
					ScrollDelta::Lines(delta) => delta.x * 20.0,
				};
				this.scroll_offset = Pixels((this.scroll_offset.0 + delta).max(0.0));
				cx.notify();
			}))
			.on_drag_move::<PanelId>(cx.listener(
				|this, event: &DragMoveEvent<PanelId>, _window, cx| {
					let dragged = *event.drag(cx);
					// Ignore drags of panels that don't belong to this strip
					// (e.g. a dock-level drag from another group passing over).
					if !this.tabs.contains(&dragged) {
						return;
					}
					if event.bounds.contains(&event.event.position) {
						if let Some(to) = this.tab_index_at(event.event.position) {
							if let Some(from) = this.tabs.iter().position(|tab| *tab == dragged) {
								this.move_tab(from, to, cx);
							}
						}
					} else {
						// The tab left the strip: hand the drag to the dock area.
						cx.emit(TabBarEvent::DockDragStarted {
							panel: dragged,
							position: event.event.position,
						});
					}
				},
			));

		for (index, &panel) in self.tabs.iter().enumerate() {
			let active = index == self.active;
			let title = self
				.titles
				.get(index)
				.cloned()
				.unwrap_or_else(|| SharedString::from("Tab"));
			let closable = self.closable.get(index).copied().unwrap_or(false);

			// Ghost shown under the pointer while this tab is being dragged.
			let ghost_title = title.clone();
			let ghost_ctor = move |_panel: &PanelId,
			                       _origin: Point<Pixels>,
			                       _window: &mut Window,
			                       cx: &mut App| {
				cx.new(|_cx| TabDragGhost {
					title: ghost_title.clone(),
				})
			};

			let mut tab = div()
				.id(ElementId::named_usize("dock-tab", panel.raw() as usize))
				.debug_selector(move || format!("dock-tab-{}", panel.raw()))
				.flex()
				.flex_row()
				.items_center()
				.min_w(px(Self::MIN_TAB_WIDTH.0))
				.px_2()
				.flex_none()
				.h_full()
				.overflow_hidden()
				.whitespace_nowrap()
				.cursor_pointer()
				.text_sm()
				.bg(if active {
					colors.selected
				} else {
					colors.background
				})
				.text_color(if active { colors.text } else { colors.disabled })
				.on_click(cx.listener(move |this, _event: &ClickEvent, window, cx| {
					this.activate(index, window, cx);
				}))
				.on_drag(panel, ghost_ctor);

			// The title fills the tab so the close button pins to its right
			// edge; truncation keeps overflowing titles from pushing it out.
			tab = tab.child(div().flex_1().min_w_0().truncate().child(title));

			if closable {
				tab = tab.child(
					div()
						.id(ElementId::named_usize(
							"dock-tab-close",
							panel.raw() as usize,
						))
						.debug_selector(move || format!("dock-tab-close-{}", panel.raw()))
						.flex_none()
						.cursor_pointer()
						.rounded_sm()
						.px_0p5()
						.text_xs()
						// Full-contrast text so the affordance is actually
						// visible next to the dimmed inactive tab label; the
						// hover surfaces the button like the app's chips.
						.text_color(colors.text)
						.hover(|style| style.bg(colors.container))
						.child("✕")
						.on_click(cx.listener(move |_this, _event: &ClickEvent, _window, cx| {
							cx.stop_propagation();
							cx.emit(TabBarEvent::CloseRequested(panel));
						})),
				);
			}

			root = root.child(tab);
		}

		root
	}
}

/// The floating view shown under the pointer while a tab is being dragged.
struct TabDragGhost {
	title: SharedString,
}

impl Render for TabDragGhost {
	fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
		let colors = cx.default_colors().clone();
		div()
			.px_2()
			.py_1()
			.rounded_md()
			.bg(colors.background)
			.border_1()
			.border_color(colors.border)
			.shadow_md()
			.child(self.title.clone())
	}
}
