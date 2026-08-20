//! Floating (undocked) panels.
//!
//! Tearing a tab out of a dock area (dropping it outside the dock) opens the
//! panel in its own OS window hosted by [`FloatingPanelWindow`]; closing that
//! window returns the panel to the dock at its original position. The window
//! lifecycle lives in [`DockArea::float_panel`](crate::dock::DockArea::float_panel):
//! it removes the panel from the layout, opens the window, and registers a
//! close hook that extracts the [`PanelHandle`] back out of the closing
//! window's root view and re-docks it. Closing the window for good (via the
//! Window menu, which has no re-dock) is signalled through a shared
//! [`std::sync::atomic`] flag set by
//! [`DockArea::close_floating`](crate::dock::DockArea::close_floating).

use crate::colors::DefaultColors;
use crate::dock::PanelHandle;
use crate::{
	Context, IntoElement, ParentElement, Pixels, Point, Render, Styled, Window, WindowBounds, div,
	px,
};

/// A window hosting a single undocked panel.
///
/// Created by [`DockArea::float_panel`](crate::dock::DockArea::float_panel)
/// when a tab is dropped outside the dock. The window renders a minimal title
/// bar (panel title) above the panel's view. The [`PanelHandle`] is stored in
/// an `Option` so it can be extracted when the window closes (see
/// [`take_panel`](FloatingPanelWindow::take_panel)) and re-docked by the
/// owning dock area instead of being dropped with the window.
pub struct FloatingPanelWindow {
	/// The panel hosted by this window, or `None` once the window is closing
	/// and the dock area has reclaimed the handle.
	panel: Option<PanelHandle>,
	/// Last known window position, mirrored into layout snapshots.
	#[allow(dead_code)] // read once floating-window geometry is persisted
	origin: Point<Pixels>,
}

impl FloatingPanelWindow {
	/// Creates the content view for a new floating window hosting `panel`.
	///
	/// The caller is responsible for opening the window with
	/// `cx.open_window` and remembering its handle so it can be closed when
	/// the panel re-docks. `initial_bounds` comes from the saved layout, or
	/// from a sensible default near the main window.
	pub fn new(
		panel: PanelHandle,
		initial_bounds: Option<WindowBounds>,
		cx: &mut Context<Self>,
	) -> Self {
		let _ = cx;
		let origin = initial_bounds
			.map(|bounds| bounds.get_bounds().origin)
			.unwrap_or_else(|| Point::new(px(0.0), px(0.0)));
		Self {
			panel: Some(panel),
			origin,
		}
	}

	/// Takes the hosted panel out of this window, so the caller can re-dock it
	/// after the window closes.
	///
	/// Called by the dock area's close hook while the window is still alive;
	/// returns `None` on a second call.
	pub fn take_panel(&mut self) -> Option<PanelHandle> {
		self.panel.take()
	}
}

impl Render for FloatingPanelWindow {
	fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
		let _ = window;
		let Some(panel) = &self.panel else {
			// The panel was reclaimed while the window was closing; render an
			// empty frame so the teardown is painless.
			return div().size_full().bg(cx.default_colors().clone().background);
		};
		let title = panel.title().clone();
		let view = panel.view().clone();
		div()
			.flex()
			.flex_col()
			.h_full()
			.bg(cx.default_colors().clone().background)
			.child(
				div()
					.flex()
					.flex_row()
					.items_center()
					.w_full()
					.px_2()
					.py_1()
					.child(div().flex_1().text_xs().truncate().child(title)),
			)
			.child(div().flex_1().min_h_0().overflow_hidden().child(view))
	}
}
