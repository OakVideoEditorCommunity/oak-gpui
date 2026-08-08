//! Dockable panel layout system: tabs, splits, and drag-to-dock.
//!
//! This module provides the building blocks for IDE/NLE-style user interfaces in
//! which the user can rearrange the workspace by dragging panels between
//! tab groups and split containers, and where the resulting layout can be
//! persisted and restored across sessions.
//!
//! # Architecture
//!
//! The layout of a [`DockArea`](crate::dock::DockArea) is an immutable-by-convention tree of
//! [`DockNode`](crate::dock::DockNode)s:
//!
//! - `Split { direction, ratio, children }` — a row or column of child nodes,
//!   sized proportionally. `ratio` is the fraction of the cross axis given to
//!   the first child; with more than two children it is the fraction given to
//!   the first child relative to the rest. See [`DockLayout::resize_split`](crate::dock::DockLayout::resize_split).
//! - `Tabs { panels, active }` — a tab group showing one panel at a time,
//!   with a tab strip (the internal `tab_bar` component) for switching,
//!   closing, and reordering.
//! - `Panel(panel_id)` — a leaf holding exactly one panel.
//!
//! Panels themselves are ordinary GPUI views that implement [`DockPanel`](crate::dock::DockPanel)
//! (on top of [`Render`](crate::Render)). They are held by the [`DockArea`](crate::dock::DockArea)
//! as type-erased [`PanelHandle`](crate::dock::PanelHandle)s keyed by [`PanelId`](crate::dock::PanelId); the tree stores only
//! ids, never views.
//!
//! # Drag to dock
//!
//! Dragging a panel by its tab (or a dedicated drag surface) starts a dock
//! drag. While dragging, every potential target offers five [`DropZone`](crate::dock::DropZone)s —
//! `Left`, `Right`, `Top`, `Bottom`, and `Center` — computed by
//! [`DockArea::drop_zone_at`](crate::dock::DockArea::drop_zone_at). Dropping on an edge zone splits the target
//! node in that direction; dropping on `Center` merges the dragged panel into
//! the target as a new tab. In addition, the outer edges of the root offer
//! drop zones that split the entire layout. A translucent drop indicator is
//! rendered above the content using [`deferred`](crate::deferred) so
//! it is not clipped by panel bounds.
//!
//! # Persistence
//!
//! Because panels are live views, only the *shape* of the tree plus stable
//! string keys for panels can be serialized. [`DockLayoutState`](crate::dock::DockLayoutState) is a
//! serde-serializable snapshot; the application supplies a [`PanelRegistry`](crate::dock::PanelRegistry)
//! that maps [`PanelId`](crate::dock::PanelId)s to string keys on save and rebuilds views from
//! those keys on restore. See [`DockLayout`](crate::dock::DockLayout) for details.
//!
//! # Wiring into your app
//!
//! The intended consumer is the Oak video editor: a media/project bin, source
//! and program monitors, an inspector, and a timeline, all dockable. The
//! typical setup is:
//!
//! 1. Implement [`DockPanel`](crate::dock::DockPanel) for each panel view (media bin, viewer,
//!    inspector, timeline, ...).
//! 2. Implement [`PanelRegistry`](crate::dock::PanelRegistry) for an application type that knows how to
//!    construct each panel from its string key.
//! 3. Create a [`DockArea`](crate::dock::DockArea), register the registry with
//!    [`DockArea::with_registry`](crate::dock::DockArea::with_registry), and seed panels with [`DockArea::add_panel`](crate::dock::DockArea::add_panel).
//! 4. On startup, call [`DockArea::restore_state`](crate::dock::DockArea::restore_state) with the previously saved
//!    [`DockLayoutState`](crate::dock::DockLayoutState) (e.g. from `serde_json`); on quit, persist
//!    [`DockArea::save_state`](crate::dock::DockArea::save_state).
//!
//! ```ignore
//! let dock = cx.new(|cx| {
//!     DockArea::new(cx)
//!         .with_registry(Arc::new(MyPanelRegistry))
//! });
//! dock.update(cx, |dock, cx| {
//!     dock.add_panel(PanelHandle::new(media_bin, cx), None, cx);
//!     dock.add_panel(PanelHandle::new(viewer, cx), None, cx);
//! });
//! ```
//!
//! See `examples/learn/dock_layout.rs` for a fuller sketch.

mod dock_area;
mod floating;
mod layout;
mod panel;
mod split_handle;
mod tab_bar;

pub use dock_area::{DockArea, DockEvent};
pub use floating::FloatingPanelWindow;
pub(crate) use layout::path_key;
pub use layout::{
    DockLayout, DockLayoutState, DockNode, DropTarget, DropZone, NodePath, PanelRegistry,
};
pub use panel::{DockPanel, PanelEvent, PanelHandle, PanelId};
