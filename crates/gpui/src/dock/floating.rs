//! Floating (undocked) panels.
//!
//! # Status
//!
//! Floating panels require spawning one OS window per floated panel, sharing
//! entities across windows, and dragging between windows. GPUI's multi-window
//! support (multiple `cx.open_window` roots sharing an [`App`](crate::App))
//! is sufficient in principle, but drag-and-drop *across* windows and
//! focus/activation semantics are unverified. Until that is proven,
//! [`DockArea::float_panel`](crate::dock::DockArea::float_panel) always
//! returns `false` and this module's view type is never constructed by the
//! dock machinery itself; it remains available so a caller can host a panel
//! in its own window.
//!
//! # Intended API
//!
//! - [`DockArea::float_panel`](crate::dock::DockArea::float_panel) removes a
//!   panel from the layout tree and opens it in its own borderless-chrome
//!   window hosting a [`FloatingPanelWindow`].
//! - [`FloatingPanelWindow`] renders the panel plus a title bar that acts as
//!   a drag surface; dropping the window back over a dock area re-docks the
//!   panel at the hovered [`DropTarget`](crate::dock::DropTarget).
//! - Floating windows are recorded in
//!   [`DockLayoutState::floating`](crate::dock::DockLayoutState::floating) so
//!   sessions restore them in place.
//!
//! Everything here is subject to change when the feature is implemented for
//! real.

use crate::colors::DefaultColors;
use crate::dock::PanelHandle;
use crate::{
	Context, IntoElement, ParentElement, Pixels, Point, Render, Styled, Window, WindowBounds, div,
	px,
};

/// A window hosting a single undocked panel.
///
/// See the [module documentation](crate::dock) — floating support is not yet
/// wired into [`DockArea`](crate::dock::DockArea), so this view is only
/// constructed by callers that host a panel in their own window.
///
/// The window renders a minimal title bar (panel title, re-dock drag surface,
/// close button) above the panel's view, and reports its bounds back to the
/// owning [`DockArea`](crate::dock::DockArea) so
/// [`DockLayoutState`](crate::dock::DockLayoutState) can restore the window
/// geometry.
pub struct FloatingPanelWindow {
	/// The panel hosted by this window.
	panel: PanelHandle,
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
		Self { panel, origin }
	}
}

impl Render for FloatingPanelWindow {
	fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
		let _ = window;
		let title = self.panel.title().clone();
		let view = self.panel.view().clone();
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
