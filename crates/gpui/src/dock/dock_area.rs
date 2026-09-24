//! The [`DockArea`] view: hosts panels, renders the layout tree, and handles
//! drag-to-dock interaction.

use crate::dock::floating::FloatingPanelWindow;
use crate::dock::layout::interim_id;
use crate::dock::panel::PanelEvent;
use crate::dock::split_handle::{SplitHandle, SplitHandleDrag, SplitHandleEvent};
use crate::dock::tab_bar::{TabBar, TabBarEvent};
use crate::dock::{
	DockLayout, DockLayoutState, DockNode, DropTarget, DropZone, NodePath, PanelHandle, PanelId,
	PanelRegistry, path_key,
};
use crate::{
	App, AppContext, Axis, Bounds, Context, Div, DragMoveEvent, ElementId, Entity, EventEmitter,
	FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Pixels, Point, Render,
	SharedString, Stateful, Styled, Subscription, Window, WindowBounds, WindowOptions, deferred,
	div, hsla, px, relative, size,
};
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Events emitted by a [`DockArea`].
///
/// Subscribe with `cx.subscribe(&dock_area, ...)` to react to layout and
/// focus changes — e.g. to update menu items or persist the layout on
/// [`DockEvent::LayoutChanged`].
#[derive(Clone, Debug)]
pub enum DockEvent {
	/// A panel was added to the dock area.
	PanelAdded(PanelId),
	/// A panel was removed from the dock area (via
	/// [`DockArea::remove_panel`]) without a close negotiation.
	PanelRemoved(PanelId),
	/// A panel became the focused panel, either by user interaction or via
	/// [`DockArea::focus_panel`].
	PanelFocused(PanelId),
	/// A panel was moved to a new position in the layout (drag-to-dock or
	/// programmatic move).
	PanelMoved {
		/// The panel that moved.
		panel: PanelId,
		/// Where it landed.
		target: DropTarget,
	},
	/// The layout tree changed shape for any reason (add, remove, move,
	/// split resize). Good trigger for autosaving
	/// [`DockArea::save_state`].
	LayoutChanged,
	/// A panel was closed by the user after the full
	/// [`should_close`](crate::dock::DockPanel::should_close) /
	/// [`on_close`](crate::dock::DockPanel::on_close) negotiation.
	PanelClosed(PanelId),
}

/// In-flight drag-to-dock state for one pointer drag.
///
/// Created when a tab drag crosses out of its tab strip (see
/// [`tab_bar`](crate::dock::tab_bar)), updated on every
/// [`DragMoveEvent`](crate::DragMoveEvent)s, and consumed or cancelled on
/// drop. Internal to [`DockArea`].
struct DockDragState {
	/// The panel being dragged.
	panel: PanelId,
	/// Current cursor position in window coordinates.
	position: Point<Pixels>,
	/// The drop target currently under the cursor, if any.
	hovered: Option<DropTarget>,
	/// Bounds of the hovered tab group / leaf, used to position the drop
	/// indicator overlay.
	hovered_bounds: Option<Bounds<Pixels>>,
}

/// One tear-off floating window, tracked so the dock can close it
/// programmatically (e.g. from the 窗口 menu) and so its close hook knows
/// whether to re-dock the panel.
struct FloatingWindowState {
	/// The window hosting the floated panel.
	window: crate::WindowHandle<FloatingPanelWindow>,
	/// Set when the shell asks to close the window *without* re-docking; the
	/// window's close hook checks it before re-inserting the panel.
	suppress_redock: Arc<AtomicBool>,
}

/// A dockable workspace: renders a [`DockLayout`] tree of panels and manages
/// docking interactions.
///
/// `DockArea` is a single GPUI view. It owns:
///
/// - the layout tree ([`DockLayout`]), edited through the methods below;
/// - the live panels, as [`PanelHandle`]s keyed by [`PanelId`];
/// - an optional [`PanelRegistry`] used by [`save_state`](DockArea::save_state)
///   / [`restore_state`](DockArea::restore_state);
/// - the in-flight drag state and drop-indicator overlay.
///
/// # Rendering
///
/// The tree is rendered recursively: `Split` nodes as flex rows/columns with
/// draggable resize handles (see the internal `split_handle` component),
/// `Tabs` nodes as a tab strip (the internal `TabBar` component) over the
/// active panel's content, and `Panel`
/// leaves as the panel's view. The drop indicator is painted in a
/// [`deferred`](crate::deferred) layer so it overlays all panels without being clipped.
///
/// # Focus and keyboard accessibility
///
/// `DockArea` implements [`Focusable`] and is itself focusable; when it holds
/// focus, keyboard commands (bound by the application) cycle focus between
/// panels (`focus_next_panel` / `focus_prev_panel` — declaration pending),
/// wrap around at the ends, and can move the focused panel with the keyboard.
/// Activating a tab also focuses its panel. Panels that implement their own
/// [`Focusable`] keep inner focus; the dock only tracks *which* panel is
/// focused via [`DockEvent::PanelFocused`].
pub struct DockArea {
	layout: DockLayout,
	panels: HashMap<PanelId, PanelHandle>,
	registry: Option<Arc<dyn PanelRegistry>>,
	focus_handle: FocusHandle,
	focused_panel: Option<PanelId>,
	drag: Option<DockDragState>,
	/// Tear-off windows (panel id → window), opened by
	/// [`float_panel`](DockArea::float_panel) and pruned when the panel
	/// re-docks or is closed for good.
	floating: HashMap<PanelId, FloatingWindowState>,
	/// Each panel's last known dock position, recorded on every layout change
	/// and before each removal so a closed/tear-off panel can be re-opened
	/// nearby (see [`last_target`](DockArea::last_target)). `None` means the
	/// panel was the root.
	last_targets: HashMap<PanelId, Option<DropTarget>>,
	/// One tab-strip entity per `Tabs` node, keyed by the node's current
	/// path. Re-created when the tree changes shape and pruned each render.
	/// The subscription keeps the strip's events routed back to this view.
	tab_bars: HashMap<NodePath, (Entity<TabBar>, Subscription)>,
	/// One split-handle entity per boundary of each `Split` node, keyed by
	/// the node's current path and the boundary index. Holds transient drag
	/// state (`SplitHandle::last_position`) across frames; pruned with the tab
	/// bars each render. A split with N children keeps N-1 handles so every
	/// pair of panels can be resized independently.
	split_handles: HashMap<(NodePath, usize), (Entity<SplitHandle>, Subscription)>,
}

impl DockArea {
	/// Creates an empty dock area with no panels and no registry.
	///
	/// Typically wrapped in an entity by the caller:
	/// `cx.new(|cx| DockArea::new(cx))`.
	pub fn new(cx: &mut Context<Self>) -> Self {
		Self {
			layout: DockLayout::new(),
			panels: HashMap::new(),
			registry: None,
			focus_handle: cx.focus_handle(),
			focused_panel: None,
			drag: None,
			floating: HashMap::new(),
			last_targets: HashMap::new(),
			tab_bars: HashMap::new(),
			split_handles: HashMap::new(),
		}
	}

	/// Builder: sets the [`PanelRegistry`] used for layout persistence.
	///
	/// Without a registry, [`save_state`](DockArea::save_state) returns an
	/// empty snapshot and [`restore_state`](DockArea::restore_state) restores
	/// nothing.
	pub fn with_registry(mut self, registry: Arc<dyn PanelRegistry>) -> Self {
		self.registry = Some(registry);
		self
	}

	/// Adds a panel at `target` (or as the root if `target` is `None` and the
	/// layout is empty).
	///
	/// Subscribes to the panel's [`PanelEvent`](crate::dock::PanelEvent)s so
	/// title changes and close requests are handled. Emits
	/// [`DockEvent::PanelAdded`] and [`DockEvent::LayoutChanged`].
	///
	/// Returns `false` (and does nothing) if a panel with the same id is
	/// already present, or if the insertion failed (see
	/// [`DockLayout::insert_panel`]).
	pub fn add_panel(
		&mut self,
		panel: PanelHandle,
		target: Option<DropTarget>,
		cx: &mut Context<Self>,
	) -> bool {
		let id = panel.panel_id();
		if self.panels.contains_key(&id) {
			return false;
		}
		if !self.layout.insert_panel(id, target) {
			return false;
		}
		let mut panel = panel;
		self.install_panel_subscription(&mut panel, cx);
		self.panels.insert(id, panel);
		self.emit_layout_changed(cx);
		cx.emit(DockEvent::PanelAdded(id));
		true
	}

	/// Removes a panel without close negotiation and returns its handle.
	///
	/// The caller regains ownership of the view (e.g. to re-dock it elsewhere
	/// or drop it). Emits [`DockEvent::PanelRemoved`] and
	/// [`DockEvent::LayoutChanged`]. Returns `None` if the id is unknown.
	/// For user-initiated closes, prefer the request flow driven by
	/// [`PanelEvent::CloseRequested`](crate::dock::PanelEvent::CloseRequested),
	/// which honors [`DockPanel::should_close`](crate::dock::DockPanel::should_close).
	pub fn remove_panel(&mut self, id: PanelId, cx: &mut Context<Self>) -> Option<PanelHandle> {
		let mut handle = self.panels.remove(&id)?;
		// Remember where the panel sat so a reopen (窗口 menu, tear-off
		// re-dock) can restore it nearby.
		self.last_targets.insert(id, self.target_of(id));
		// Dropping the subscription unsubscribes from the panel's events.
		handle.set_subscription(None);
		self.layout.remove_panel(id);
		if self.focused_panel == Some(id) {
			self.focused_panel = None;
		}
		self.emit_layout_changed(cx);
		cx.emit(DockEvent::PanelRemoved(id));
		Some(handle)
	}

	/// Moves keyboard focus (and the tab-strip selection) to `panel`.
	///
	/// Activates the panel's tab if it lives in a `Tabs` group, focuses the
	/// panel's own focus handle if it has one, and emits
	/// [`DockEvent::PanelFocused`]. No-op if the id is unknown.
	pub fn focus_panel(&mut self, id: PanelId, window: &mut Window, cx: &mut Context<Self>) {
		if !self.panels.contains_key(&id) {
			return;
		}
		// Activate the panel's tab so the strip selection follows.
		if let Some(path) = self.layout.find_panel(id) {
			self.layout.set_tabs_active(&path, id);
		}
		self.focused_panel = Some(id);
		// The panel's own focus handle is not reachable through the
		// type-erased `AnyView`, so give the dock area keyboard focus; panels
		// that implement `Focusable` re-establish inner focus on interaction.
		self.focus_handle.focus(window, cx);
		cx.emit(DockEvent::PanelFocused(id));
		cx.notify();
	}

	/// Returns the current layout tree.
	pub fn layout(&self) -> &DockLayout {
		&self.layout
	}

	/// Replaces the whole layout tree.
	///
	/// Panels referenced by `layout` that have no registered handle are
	/// dropped from the tree (via [`DockLayout::cleanup`]); panels that are
	/// registered but unreferenced stay loaded but hidden until re-added.
	/// Emits [`DockEvent::LayoutChanged`].
	pub fn set_layout(&mut self, mut layout: DockLayout, cx: &mut Context<Self>) {
		// Panels the caller's tree references without a live handle are
		// dropped; `remove_panel` runs `cleanup` to collapse the gaps.
		let missing: Vec<PanelId> = layout
			.panels()
			.into_iter()
			.filter(|id| !self.panels.contains_key(id))
			.collect();
		for id in missing {
			layout.remove_panel(id);
		}
		self.layout = layout;
		self.emit_layout_changed(cx);
	}

	/// Returns the panel handle for `id`, if registered.
	pub fn panel(&self, id: PanelId) -> Option<&PanelHandle> {
		self.panels.get(&id)
	}

	/// Re-reads every held panel's (localized) title into its cached
	/// snapshot, marks each panel view dirty so its content re-renders, and
	/// repaints the dock chrome.
	///
	/// Call this after switching the UI language: [`DockPanel::title`] and
	/// the panel bodies are localized, but the tab strip reads the cached
	/// snapshot and the panels only re-render when notified.
	pub fn refresh_panel_titles(&mut self, cx: &mut Context<Self>) {
		for handle in self.panels.values_mut() {
			handle.refresh_title(cx);
			handle.notify_panel(cx);
		}
		cx.notify();
	}

	/// Captures a serializable snapshot of the current layout.
	///
	/// Requires a registry (see [`with_registry`](DockArea::with_registry));
	/// without one, returns an empty snapshot. Panels the registry declines
	/// to key are omitted. Persist with `serde_json` or similar.
	pub fn save_state(&self) -> DockLayoutState {
		match &self.registry {
			Some(registry) => DockLayoutState::capture(&self.layout, registry.as_ref()),
			// Without a registry no panel can be keyed, so the snapshot is
			// empty (but carries the current format version).
			None => DockLayoutState::capture(&self.layout, &NoopPanelRegistry),
		}
	}

	/// Restores a previously saved snapshot, rebuilding panels through the
	/// registry.
	///
	/// Panels whose keys the registry cannot rebuild are skipped; the layout
	/// is normalized afterwards. Existing panels not referenced by the
	/// snapshot are kept registered (hidden) so their state survives a
	/// layout switch; panels that *are* referenced are re-used rather than
	/// rebuilt when their current id's key matches.
	///
	/// Emits [`DockEvent::LayoutChanged`] if the tree changed.
	///
	/// # Panics
	///
	/// Does not panic on malformed input; unknown keys and bad indices are
	/// dropped/clamped. Returns without effect if no registry is set.
	pub fn restore_state(
		&mut self,
		state: &DockLayoutState,
		window: &mut Window,
		cx: &mut Context<Self>,
	) {
		let Some(registry) = self.registry.clone() else {
			return;
		};
		// The tree shape with panels addressed by deterministic interim ids
		// (hashed from the registry keys); real ids are resolved below.
		let mut layout = state.to_layout();

		// Rebuild the panels, mapping each key's interim id to the live
		// panel's id. Re-use an already-registered panel when its key matches
		// so per-panel state survives the layout switch.
		let mut interim_to_real: HashMap<PanelId, PanelId> = HashMap::new();
		let mut fresh: Vec<PanelHandle> = Vec::new();
		for key in state.keys() {
			let interim = interim_id(&key);
			if interim_to_real.contains_key(&interim) {
				// The same key occurred twice in the snapshot; the duplicate
				// occurrence is dropped when the tree is re-mapped below.
				continue;
			}
			let reuse = self
				.panels
				.values()
				.find(|handle| registry.panel_key(handle.panel_id()) == Some(key.clone()));
			match reuse {
				Some(handle) => {
					interim_to_real.insert(interim, handle.panel_id());
				}
				None => match registry.build_panel(&key, window, cx) {
					Some(handle) => {
						interim_to_real.insert(interim, handle.panel_id());
						fresh.push(handle);
					}
					None => {} // unknown key; its node collapses away
				},
			}
		}

		// Re-write the interim ids to real ids, dropping panels that could
		// not be rebuilt and de-duplicating panels that appear more than once.
		let mut used: HashSet<PanelId> = HashSet::new();
		let mut to_remove: Vec<PanelId> = Vec::new();
		if let Some(root) = layout.root_mut() {
			Self::remap_node_ids(root, &interim_to_real, &mut used, &mut to_remove);
		}
		for id in to_remove {
			layout.remove_panel(id);
		}

		self.layout = layout;
		for handle in fresh {
			let id = handle.panel_id();
			if self.panels.contains_key(&id) {
				continue;
			}
			let mut handle = handle;
			self.install_panel_subscription(&mut handle, cx);
			self.panels.insert(id, handle);
		}
		self.emit_layout_changed(cx);
	}

	/// Undocks a panel into its own floating window.
	///
	/// Removes the panel from the layout (through the same
	/// [`remove_panel`](DockArea::remove_panel) flow the tab close button uses,
	/// so the position is recorded for a later re-dock) and opens a
	/// [`FloatingPanelWindow`](crate::dock::FloatingPanelWindow) hosting it.
	/// The window's close hook reclaims the [`PanelHandle`] out of the closing
	/// window and re-docks it at the panel's original position; closing the
	/// window for good (e.g. via the 窗口 menu, see
	/// [`close_floating`](DockArea::close_floating)) skips the re-dock.
	///
	/// Returns `false` if the panel is not docked, or already floating.
	///
	/// The window is opened from a deferred context by the caller paths
	/// ([`finish_drag`](DockArea::finish_drag) / the render-time stale-drag
	/// cleanup) because opening a window from inside a mouse-event dispatch
	/// can re-enter the app update.
	pub fn float_panel(&mut self, id: PanelId, cx: &mut Context<Self>) -> bool {
		if self.floating.contains_key(&id) || !self.panels.contains_key(&id) {
			return false;
		}
		let Some(handle) = self.remove_panel(id, cx) else {
			return false;
		};
		let title = handle.title().clone();
		let suppress = Arc::new(AtomicBool::new(false));
		let suppress_close = suppress.clone();
		let dock = cx.weak_entity();
		let floating = cx.new(|cx| FloatingPanelWindow::new(handle, None, cx));
		let root = floating.clone();
		let close_root = floating.clone();
		let bounds = Bounds::centered(None, size(px(640.0), px(480.0)), cx);
		let window = cx.open_window(
			WindowOptions {
				window_bounds: Some(WindowBounds::Windowed(bounds)),
				titlebar: Some(crate::TitlebarOptions {
					title: Some(title.clone()),
					appears_transparent: false,
					traffic_light_position: None,
				}),
				..Default::default()
			},
			move |window, app| {
				// Re-dock on close: while the window is still alive, lift the
				// panel out of its root view and hand it back to the dock.
				window.on_window_should_close(app, move |_window, app| {
					if suppress_close.load(Ordering::SeqCst) {
						// Explicit close (窗口 menu): the panel stays closed.
						return true;
					}
					let panel = close_root.update(app, |floating, _| floating.take_panel());
					let Some(panel) = panel else {
						return true;
					};
					if let Some(dock) = dock.upgrade() {
						let _ = dock.update(app, |dock, cx| dock.redock(panel, cx));
					}
					true
				});
				root
			},
		);
		let Ok(window) = window else {
			// Could not open the window; put the panel straight back so it is
			// never lost.
			let handle = floating.update(cx, |floating, _| floating.take_panel());
			if let Some(handle) = handle {
				self.redock(handle, cx);
			}
			return false;
		};
		self.floating.insert(
			id,
			FloatingWindowState {
				window,
				suppress_redock: suppress,
			},
		);
		true
	}

	/// Closes a floating window *without* re-docking the panel (the 窗口 menu
	/// uses this to fully close a torn-off panel). No-op if the panel is not
	/// floating.
	pub fn close_floating(&mut self, id: PanelId, cx: &mut Context<Self>) {
		let Some(state) = self.floating.remove(&id) else {
			return;
		};
		state.suppress_redock.store(true, Ordering::SeqCst);
		let _ = state.window.update(cx, |_, window, _| window.remove_window());
	}

	/// Re-docks a panel returned by a closing floating window, at the position
	/// it had before it was floated.
	fn redock(&mut self, panel: PanelHandle, cx: &mut Context<Self>) {
		let id = panel.panel_id();
		self.floating.remove(&id);
		let target = self.fallback_target(self.last_target(id));
		let _ = self.add_panel(panel, target, cx);
	}

	/// Returns the last recorded dock position of `panel` (where it sat before
	/// its most recent removal, refreshed on every layout change), or `None`
	/// if it was the root / has never been docked.
	pub fn last_target(&self, id: PanelId) -> Option<DropTarget> {
		self.last_targets.get(&id).copied().flatten()
	}

	/// Whether `panel` is currently docked in the layout.
	pub fn is_docked(&self, id: PanelId) -> bool {
		self.layout.contains(id)
	}

	/// Whether `panel` is currently shown in a floating (tear-off) window.
	pub fn is_floating(&self, id: PanelId) -> bool {
		self.floating.contains_key(&id)
	}

	/// Whether `panel` is currently visible anywhere: docked or floating.
	pub fn is_panel_visible(&self, id: PanelId) -> bool {
		self.is_docked(id) || self.is_floating(id)
	}

	/// Resolves `target` to a usable drop target for re-inserting a panel: if
	/// the anchor panel is no longer in the layout, falls back to merging into
	/// the first panel that is; `None` (the root) is kept for an empty layout.
	pub fn fallback_target(&self, target: Option<DropTarget>) -> Option<DropTarget> {
		match target {
			Some(t) if t.panel.map_or(true, |anchor| self.layout.contains(anchor)) => target,
			Some(_) => self.layout.panels().first().map(|&anchor| DropTarget {
				panel: Some(anchor),
				zone: DropZone::Center,
			}),
			None => None,
		}
	}

	/// Returns a drop target that would re-insert `panel` at roughly its
	/// current position in the layout (its tab group, or beside its split
	/// neighbor), so a closed or floated panel can be re-opened nearby.
	/// `None` when the panel is the root.
	pub fn target_of(&self, panel: PanelId) -> Option<DropTarget> {
		let root = self.layout.root()?;
		Self::target_of_in(root, panel, None)
	}

	/// The first panel id in the subtree rooted at `node`, depth-first.
	fn first_panel(node: &DockNode) -> Option<PanelId> {
		match node {
			DockNode::Panel(id) => Some(*id),
			DockNode::Tabs { panels, .. } => panels.first().copied(),
			DockNode::Split { children, .. } => children.iter().find_map(Self::first_panel),
		}
	}

	/// Recursive half of [`target_of`](DockArea::target_of); `parent_split`
	/// is the split node the current node is a child of, with its index.
	fn target_of_in(
		node: &DockNode,
		panel: PanelId,
		parent_split: Option<(&DockNode, usize)>,
	) -> Option<DropTarget> {
		match node {
			DockNode::Panel(id) if *id == panel => {
				let (split, index) = parent_split?;
				let DockNode::Split { children, .. } = split else {
					unreachable!("a Panel leaf's parent is a Split (or the root)")
				};
				// Re-insert beside the nearest sibling, using the zone that
				// puts the panel on its original side of the neighbor.
				if let Some(sibling) = children.get(index + 1) {
					Self::first_panel(sibling).map(|anchor| DropTarget {
						panel: Some(anchor),
						zone: DropZone::Left,
					})
				} else if index > 0 {
					children.get(index - 1).and_then(Self::first_panel).map(|anchor| {
						DropTarget {
							panel: Some(anchor),
							zone: DropZone::Right,
						}
					})
				} else {
					None
				}
			}
			DockNode::Panel(_) => None,
			DockNode::Tabs { panels, .. } => {
				if panels.contains(&panel) {
					let anchor = panels
						.iter()
						.find(|&&other| other != panel)
						.copied()
						.or_else(|| panels.first().copied())?;
					Some(DropTarget {
						panel: Some(anchor),
						zone: DropZone::Center,
					})
				} else {
					None
				}
			}
			DockNode::Split { children, .. } => {
				for (index, child) in children.iter().enumerate() {
					if let Some(target) = Self::target_of_in(child, panel, Some((node, index))) {
						return Some(target);
					}
				}
				None
			}
		}
	}

	/// Hit-tests a cursor position against the five drop zones of a target.
	///
	/// `point` is in window coordinates; `target_bounds` are the bounds of
	/// the hovered tab group / leaf. The bounds are divided into a center
	/// region ([`DropZone::Center`]) and four edge bands; the band width is a
	/// fraction of the smaller dimension so narrow targets stay usable.
	/// Returns `None` when `point` lies outside `target_bounds`.
	///
	/// Pure and exposed for testing; the drag handlers call it with the
	/// bounds cached in the internal drag state.
	pub fn drop_zone_at(point: Point<Pixels>, target_bounds: Bounds<Pixels>) -> Option<DropZone> {
		if !target_bounds.contains(&point) {
			return None;
		}
		let width = target_bounds.size.width.0;
		let height = target_bounds.size.height.0;
		// Edge bands are a quarter of the smaller dimension, so narrow
		// targets (deeply split columns) keep usable edge zones.
		let band = (width.min(height) * 0.25).max(1.0);
		let dx = point.x.0 - target_bounds.origin.x.0;
		let dy = point.y.0 - target_bounds.origin.y.0;
		if dx < band {
			Some(DropZone::Left)
		} else if dx > width - band {
			Some(DropZone::Right)
		} else if dy < band {
			Some(DropZone::Top)
		} else if dy > height - band {
			Some(DropZone::Bottom)
		} else {
			Some(DropZone::Center)
		}
	}

	/// Renders the translucent drop indicator for the currently hovered
	/// [`DropTarget`], if a drag is in flight.
	///
	/// Painted via [`deferred`](crate::deferred) so it overlays panel content unclipped. The
	/// indicator highlights the sub-rectangle of the target bounds that the
	/// panel would occupy (half for edge zones, full for
	/// [`DropZone::Center`]).
	fn render_drop_indicator(
		&self,
		_window: &mut Window,
		_cx: &mut Context<Self>,
	) -> Option<impl IntoElement> {
		let drag = self.drag.as_ref()?;
		let (target, bounds) = (drag.hovered?, drag.hovered_bounds?);
		// The hovered bounds are in window coordinates; the dock area's root
		// is laid out at the window origin in its primary embedding, so they
		// double as root-relative coordinates for the absolutely positioned
		// overlay. Display-only — docking hit-testing never depends on it.
		let rect = match target.zone {
			DropZone::Center => bounds,
			DropZone::Left => Bounds::new(
				bounds.origin,
				size(bounds.size.width * 0.5, bounds.size.height),
			),
			DropZone::Right => Bounds::new(
				Point::new(bounds.right() - bounds.size.width * 0.5, bounds.top()),
				size(bounds.size.width * 0.5, bounds.size.height),
			),
			DropZone::Top => Bounds::new(
				bounds.origin,
				size(bounds.size.width, bounds.size.height * 0.5),
			),
			DropZone::Bottom => Bounds::new(
				Point::new(bounds.left(), bounds.bottom() - bounds.size.height * 0.5),
				size(bounds.size.width, bounds.size.height * 0.5),
			),
		};
		Some(deferred(
			div()
				.absolute()
				.left(px(rect.origin.x.0))
				.top(px(rect.origin.y.0))
				.w(px(rect.size.width.0))
				.h(px(rect.size.height.0))
				.rounded_md()
				.bg(hsla(0.62, 0.7, 0.8, 0.25)),
		))
	}

	/// Begins a dock drag for `panel`. Called by the tab strip when a tab
	/// drag leaves the strip's bounds.
	fn begin_drag(&mut self, panel: PanelId, position: Point<Pixels>, cx: &mut Context<Self>) {
		match &mut self.drag {
			// Re-entered the strip mid-drag (the strip keeps emitting
			// `DockDragStarted`); keep the hovered target intact.
			Some(drag) if drag.panel == panel => {
				drag.position = position;
			}
			_ => {
				self.drag = Some(DockDragState {
					panel,
					position,
					hovered: None,
					hovered_bounds: None,
				});
			}
		}
		cx.notify();
	}

	/// Ensures a drag state exists for `panel`, creating one lazily when a
	/// `PanelId` drag arrives at this dock area.
	///
	/// The tab strip hands the drag over via the deferred
	/// [`TabBarEvent::DockDragStarted`] subscription, which is only delivered
	/// after the current mouse-event dispatch completes — so a fast drag can
	/// cross the strip and reach its target before the state exists, and the
	/// hovered target (and thus the drop) is missed. The dock area's own
	/// capture-phase `on_drag_move` handlers run first in the same dispatch, so
	/// they call this to start the state immediately and never depend on the
	/// strip's (redundant) handoff. Drags of panels this dock does not own are
	/// ignored.
	fn ensure_drag(&mut self, panel: PanelId, position: Point<Pixels>, cx: &mut Context<Self>) {
		match &mut self.drag {
			Some(drag) if drag.panel == panel => {
				drag.position = position;
			}
			_ if self.panels.contains_key(&panel) => {
				self.drag = Some(DockDragState {
					panel,
					position,
					hovered: None,
					hovered_bounds: None,
				});
			}
			_ => return,
		}
		cx.notify();
	}

	/// Updates the hovered drop target during a drag. Attached to panel
	/// containers via `on_drag_move` with a payload identifying the dragged
	/// panel.
	///
	/// The calling handler stashes the candidate target (which panel, or
	/// `None` for the dock area's outer edges) and its bounds in the drag
	/// state; this method classifies the cursor within those bounds and
	/// commits (or clears) the hovered target.
	fn update_drag(&mut self, position: Point<Pixels>, _window: &Window, cx: &mut Context<Self>) {
		let Some(drag) = self.drag.as_mut() else {
			return;
		};
		let Some(bounds) = drag.hovered_bounds else {
			return;
		};
		drag.position = position;
		let panel = drag.hovered.and_then(|target| target.panel);
		match Self::drop_zone_at(position, bounds) {
			// A root-edge target (`panel: None`) may only ever be an edge
			// zone; a center hit falls through to the no-target case.
			Some(zone) if panel.is_some() || zone.is_split() => {
				drag.hovered = Some(DropTarget { panel, zone });
			}
			_ => {
				drag.hovered = None;
				drag.hovered_bounds = None;
			}
		}
		cx.notify();
	}

	/// Completes the current drag, applying the hovered drop.
	///
	/// No-op if no drag is in flight or nothing is hovered; emits
	/// [`DockEvent::PanelMoved`] and [`DockEvent::LayoutChanged`] on success.
	fn finish_drag(&mut self, cx: &mut Context<Self>) {
		let Some(drag) = self.drag.take() else {
			return;
		};
		let Some(target) = drag.hovered else {
			cx.notify();
			return;
		};
		if self.layout.move_panel(drag.panel, target) {
			cx.emit(DockEvent::PanelMoved {
				panel: drag.panel,
				target,
			});
			self.emit_layout_changed(cx);
		} else {
			cx.notify();
		}
	}

	/// Emits [`DockEvent::LayoutChanged`] and repaints. Also refreshes each
	/// panel's recorded dock position ([`last_target`](DockArea::last_target))
	/// so a subsequent close can restore it.
	fn emit_layout_changed(&mut self, cx: &mut Context<Self>) {
		for id in self.layout.panels() {
			self.last_targets.insert(id, self.target_of(id));
		}
		cx.emit(DockEvent::LayoutChanged);
		cx.notify();
	}

	/// Installs the subscription routing a panel's [`PanelEvent`]s back to
	/// this dock area. The subscription is stored on the handle so it is
	/// dropped (and unsubscribed) when the panel leaves the dock.
	fn install_panel_subscription(&mut self, handle: &mut PanelHandle, cx: &mut Context<Self>) {
		let id = handle.panel_id();
		// Panels are type-erased (`AnyView`), so a typed `Context::subscribe`
		// cannot reach them; subscribe by entity id and downcast the event.
		let entity_id = handle.view().entity_id();
		let this = cx.weak_entity();
		let subscription = cx.new_subscription(
			entity_id,
			(
				std::any::TypeId::of::<PanelEvent>(),
				Box::new(move |event: &dyn Any, cx: &mut App| {
					let event = event
						.downcast_ref::<PanelEvent>()
						.expect("dock panel events are PanelEvent");
					let Some(entity) = this.upgrade() else {
						return false;
					};
					entity.update(cx, |dock, cx| dock.on_panel_event(id, event.clone(), cx));
					true
				}),
			),
		);
		handle.set_subscription(Some(subscription));
	}

	/// Handles a [`PanelEvent`] emitted by a held panel.
	fn on_panel_event(&mut self, id: PanelId, event: PanelEvent, cx: &mut Context<Self>) {
		match event {
			PanelEvent::CloseRequested => {
				// Panels initiate the close flow themselves — consulting
				// `DockPanel::should_close` is not reachable through the
				// type-erased view — so a request is a confirmed close.
				let _ = self.remove_panel(id, cx);
			}
			PanelEvent::Focused => {
				self.focused_panel = Some(id);
				cx.emit(DockEvent::PanelFocused(id));
				cx.notify();
			}
			PanelEvent::TitleChanged => {
				// The cached title snapshot on the handle cannot be refreshed
				// through `AnyView`; repaint so the strip re-reads what it can.
				cx.notify();
			}
		}
	}

	/// Handles an event emitted by one of the tab-strip entities.
	fn on_tab_bar_event(&mut self, event: &TabBarEvent, cx: &mut Context<Self>) {
		match event {
			TabBarEvent::Reordered { tabs, active } => {
				// Paths captured at subscribe time go stale on structural
				// edits, so resolve the owning `Tabs` node from the panel set
				// the strip just reported.
				let Some(first) = tabs.first() else {
					return;
				};
				let Some(path) = self.layout.find_panel(*first) else {
					return;
				};
				let Some(node) = self.layout.node_at_mut(&path) else {
					return;
				};
				if let DockNode::Tabs {
					panels,
					active: current,
				} = node && panels.len() == tabs.len()
				{
					*panels = tabs.clone();
					*current = *active;
					cx.notify();
				}
			}
			TabBarEvent::CloseRequested(id) => {
				let _ = self.remove_panel(*id, cx);
			}
			TabBarEvent::DockDragStarted { panel, position } => {
				self.begin_drag(*panel, *position, cx);
			}
		}
	}

	/// Gets (creating and subscribing on first use) the tab strip for the
	/// `Tabs` node at `path`, and syncs it with the node's current state.
	fn tab_bar_for(
		&mut self,
		path: &NodePath,
		panels: &[PanelId],
		active: usize,
		cx: &mut Context<Self>,
	) -> Entity<TabBar> {
		let titles: Vec<SharedString> = panels
			.iter()
			.map(|id| {
				self.panels
					.get(id)
					.map(|handle| handle.title().clone())
					.unwrap_or_default()
			})
			.collect();
		let closable: Vec<bool> = panels
			.iter()
			.map(|id| {
				self.panels
					.get(id)
					.map(|handle| handle.closable())
					.unwrap_or(false)
			})
			.collect();
		let bar = match self.tab_bars.get(path) {
			Some((bar, _)) => bar.clone(),
			None => {
				let bar = cx.new(|_cx| TabBar::new(panels.to_vec(), active));
				let subscription = cx.subscribe(&bar, |this, _bar, event: &TabBarEvent, cx| {
					this.on_tab_bar_event(event, cx);
				});
				self.tab_bars
					.insert(path.clone(), (bar.clone(), subscription));
				bar
			}
		};
		bar.update(cx, |bar, cx| {
			bar.sync(panels, active, &titles, &closable, cx)
		});
		bar
	}

	/// Gets (creating and subscribing on first use) the split-handle entity
	/// for the boundary at `index` of the `Split` node at `path`.
	fn split_handle_for(
		&mut self,
		path: &NodePath,
		index: usize,
		direction: Axis,
		cx: &mut Context<Self>,
	) -> Entity<SplitHandle> {
		if let Some((handle, _)) = self.split_handles.get(&(path.clone(), index)) {
			return handle.clone();
		}
		let handle = cx.new(|_cx| SplitHandle::new(direction, path.clone(), index));
		let subscription =
			cx.subscribe(
				&handle,
				|this, _handle, event: &SplitHandleEvent, cx| match event {
					SplitHandleEvent::ResizeRequested { path, index, ratio } => {
						this.layout.resize_split_child(path, *index, *ratio);
						this.emit_layout_changed(cx);
					}
					SplitHandleEvent::ResetRequested { path, index } => {
						this.layout
							.resize_split_child(path, *index, SplitHandle::RESET_RATIO);
						this.emit_layout_changed(cx);
					}
				},
			);
		self.split_handles
			.insert((path.clone(), index), (handle.clone(), subscription));
		handle
	}

	/// Routes a split-handle drag to the handle entity for the boundary at
	/// `index` of the split at `path`.
	fn route_split_drag(
		&mut self,
		path: &NodePath,
		index: usize,
		direction: Axis,
		event: &DragMoveEvent<SplitHandleDrag>,
		cx: &mut Context<Self>,
	) {
		let Some((handle, _)) = self.split_handles.get(&(path.clone(), index)) else {
			return;
		};
		// The handle drag works on the pair's own extent: start_ratio is the
		// index child's share of the pair, and the drag delta is a fraction
		// of the pair's combined on-screen extent.
		let ratios = self.layout.split_ratios(path).unwrap_or_default();
		let total: f32 = ratios.iter().sum();
		let pair_total = ratios.get(index).copied().unwrap_or(0.0)
			+ ratios.get(index + 1).copied().unwrap_or(0.0);
		let start_ratio = if pair_total > 0.0 {
			ratios[index] / pair_total
		} else {
			SplitHandle::RESET_RATIO
		};
		let full_extent = match direction {
			Axis::Horizontal => event.bounds.size.width,
			Axis::Vertical => event.bounds.size.height,
		};
		// The children share the container extent minus the fixed-size
		// handles between them; the pair's ratio share applies to that
		// remainder (keeps the divider tracking the pointer 1:1).
		let handles = ratios.len().saturating_sub(1) as f32;
		let usable = full_extent - SplitHandle::HITBOX * handles;
		let pair_extent = usable * pair_total / total.max(1.0);
		let position = match direction {
			Axis::Horizontal => event.event.position.x,
			Axis::Vertical => event.event.position.y,
		};
		let handle = handle.clone();
		handle.update(cx, |handle, cx| {
			handle.drag_to(position, pair_extent, start_ratio, cx)
		});
	}

	/// Ends a split-handle drag on the handle entity for the boundary at
	/// `index` of the split at `path`.
	fn end_split_drag(&mut self, path: &NodePath, drag: &SplitHandleDrag, cx: &mut Context<Self>) {
		if let Some((handle, _)) = self.split_handles.get(&(path.clone(), drag.index)) {
			let handle = handle.clone();
			handle.update(cx, |handle, _cx| handle.end_drag());
		}
	}

	/// Claims the hovered drop target for a panel container (`target` is the
	/// panel, or the whole tab group, the container belongs to).
	///
	/// Attached to every rendered panel container via `on_drag_move`; the
	/// container's bounds come from the drag event itself, so no element
	/// lookup is needed. Runs after the root's own `on_drag_move` (capture
	/// phase), overriding any root-edge target with the exact per-panel one —
	/// except over the dragged panel's own group, where the drop is a no-op and
	/// a root-edge target is left intact so the panel can still be docked to
	/// the outer edge of the whole area.
	fn update_panel_drag(
		&mut self,
		target: &[PanelId],
		event: &DragMoveEvent<PanelId>,
		window: &Window,
		cx: &mut Context<Self>,
	) {
		let dragged = *event.drag(cx);
		// Lazily start the drag state; the strip's own handoff arrives one
		// dispatch late (see `ensure_drag`).
		self.ensure_drag(dragged, event.event.position, cx);
		if target.contains(&dragged) {
			// Dropping a panel onto itself or its own tab group is a no-op;
			// clear the hovered target. A root-edge target (`panel: None`)
			// established by the root handler for this exact position still
			// wins — it docks the panel to the outer edge of the whole area
			// rather than back into its own group.
			let keep_root_edge = self
				.drag
				.as_ref()
				.and_then(|drag| drag.hovered)
				.is_some_and(|target| target.panel.is_none());
			if !keep_root_edge {
				if let Some(drag) = self.drag.as_mut() {
					drag.hovered = None;
					drag.hovered_bounds = None;
				}
			}
			return;
		}
		let Some(drag) = self.drag.as_mut() else {
			return;
		};
		if drag.panel != dragged {
			return;
		}
		let target_panel = target.first().copied().unwrap_or(dragged);
		drag.hovered = Some(DropTarget {
			panel: Some(target_panel),
			zone: DropZone::Center,
		});
		drag.hovered_bounds = Some(event.bounds);
		self.update_drag(event.event.position, window, cx);
	}

	/// Renders the subtree for `node` (at `path`), creating and syncing the
	/// per-node chrome (tab bars, split handles) as it goes.
	fn render_node(
		&mut self,
		node: &DockNode,
		path: &NodePath,
		cx: &mut Context<Self>,
	) -> Stateful<Div> {
		match node {
			DockNode::Split {
				direction,
				ratios,
				children,
			} => {
				let direction = *direction;
				// This container's own path, so the drag-move handler only
				// routes drags that belong to this split (a drag-move fires on
				// every split container while any handle is dragged; nested
				// splits must not steal or double-apply the resize).
				let container_path = path.clone();
				let mut container = div()
					.flex()
					.size_full()
					.overflow_hidden()
					.id(ElementId::named_usize("dock-split", path_key(path)))
					.on_drag_move::<SplitHandleDrag>(cx.listener(
						move |this, event: &DragMoveEvent<SplitHandleDrag>, _window, cx| {
							let drag = event.drag(cx);
							if drag.path != container_path {
								return;
							}
							let path = drag.path.clone();
							let index = drag.index;
							this.route_split_drag(&path, index, direction, event, cx);
						},
					))
					.on_drop::<SplitHandleDrag>(cx.listener(
						move |this, drag: &SplitHandleDrag, _window, cx| {
							this.end_split_drag(&drag.path, drag, cx);
						},
					));
				match direction {
					Axis::Horizontal => {
						container = container.flex_row();
					}
					Axis::Vertical => {
						container = container.flex_col();
					}
				}
				for (index, child) in children.iter().enumerate() {
					let mut child_path = path.clone();
					child_path.0.push(index);
					let child = self.render_node(child, &child_path, cx);
					// Each child is sized by its own ratio (the entries sum to
					// 1.0), so a split with three or more panels keeps
					// distinct sizes instead of flattening to one ratio.
					let share = ratios
						.get(index)
						.copied()
						.unwrap_or(1.0 / children.len() as f32);
					let child = child
						.flex_basis(relative(share))
						.flex_grow_0()
						.flex_shrink();
					container = container.child(child);
					// One handle per boundary, so every pair of panels can be
					// resized independently.
					if index + 1 < children.len() {
						let handle = self.split_handle_for(path, index, direction, cx);
						container = container.child(handle);
					}
				}
				container
			}
			DockNode::Tabs { panels, active } => {
				let active = (*active).min(panels.len().saturating_sub(1));
				let active_panel = panels[active];
				let target = panels.clone();
				let mut container = div()
					.size_full()
					.flex()
					.flex_col()
					.overflow_hidden()
					.id(ElementId::named_usize(
						"dock-panel",
						active_panel.raw() as usize,
					))
					.on_drag_move::<PanelId>(cx.listener(
						move |this, event: &DragMoveEvent<PanelId>, window, cx| {
							this.update_panel_drag(&target, event, window, cx);
						},
					))
					.on_drop::<PanelId>(cx.listener(|this, _drag: &PanelId, _window, cx| {
						// The root's own drop listener is shadowed by panel
						// hitboxes, so drops over panel content land here.
						this.finish_drag(cx);
					}));
				let bar = self.tab_bar_for(path, panels, active, cx);
				container = container.child(bar);
				let content = self
					.panels
					.get(&active_panel)
					.map(|handle| handle.view().clone());
				let mut content_wrapper = div().flex_1().min_h_0().overflow_hidden();
				if let Some(view) = content {
					content_wrapper = content_wrapper.child(view);
				}
				container = container.child(content_wrapper);
				container
			}
			DockNode::Panel(id) => {
				let panel_id = *id;
				let mut container = div()
					.size_full()
					.overflow_hidden()
					.id(ElementId::named_usize(
						"dock-panel",
						panel_id.raw() as usize,
					))
					.on_drag_move::<PanelId>(cx.listener(
						move |this, event: &DragMoveEvent<PanelId>, window, cx| {
							this.update_panel_drag(&[panel_id], event, window, cx);
						},
					))
					.on_drop::<PanelId>(cx.listener(|this, _drag: &PanelId, _window, cx| {
						this.finish_drag(cx);
					}));
				if let Some(view) = self
					.panels
					.get(&panel_id)
					.map(|handle| handle.view().clone())
				{
					container = container.child(view);
				}
				container
			}
		}
	}
}

impl Render for DockArea {
	fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
		// A drag that ended without a drop — released outside the dock area
		// (or the window) so no `on_drop` ran — leaves transient drag state
		// behind. That release is the tear-off gesture: float the panel into
		// its own window. The float is deferred out of the render pass, which
		// must stay side-effect free.
		if self.drag.is_some() && !cx.has_active_drag() {
			if let Some(drag) = self.drag.take() {
				let panel = drag.panel;
				let this = cx.weak_entity();
				cx.defer(move |app| {
					if let Some(this) = this.upgrade() {
						this.update(app, |this, cx| {
							this.float_panel(panel, cx);
						});
					}
				});
			}
		}

		let mut root = div()
			.size_full()
			.relative()
			.id("dock-area")
			// Root-edge drop zones: hovering the outer band of the whole area
			// splits the entire layout (target.panel: None). Per-panel
			// containers run after this in the capture phase and override.
			.on_drag_move::<PanelId>(cx.listener(
				|this, event: &DragMoveEvent<PanelId>, window, cx| {
					let dragged = *event.drag(cx);
					// Lazily start the drag state; the strip's own handoff
					// arrives one dispatch late (see `ensure_drag`).
					this.ensure_drag(dragged, event.event.position, cx);
					let Some(drag) = this.drag.as_mut() else {
						return;
					};
					if drag.panel != dragged {
						return;
					}
					let candidate = match DockArea::drop_zone_at(event.event.position, event.bounds)
					{
						Some(zone) if zone.is_split() => Some(DropTarget { panel: None, zone }),
						_ => None,
					};
					drag.hovered = candidate;
					drag.hovered_bounds = Some(event.bounds);
					this.update_drag(event.event.position, window, cx);
				},
			))
			.on_drop::<PanelId>(cx.listener(|this, _drag: &PanelId, _window, cx| {
				this.finish_drag(cx);
			}));

		if let Some(node) = self.layout.root().cloned() {
			let path = NodePath::default();
			root = root.child(self.render_node(&node, &path, cx));
		}

		// Drop chrome for nodes that no longer exist (their paths went stale
		// after a structural edit); this also releases their subscriptions.
		let mut live: HashSet<NodePath> = HashSet::new();
		self.collect_paths(&self.layout, &NodePath::default(), &mut live);
		self.tab_bars.retain(|path, _| live.contains(path));
		self.split_handles
			.retain(|(path, _), _| live.contains(path));

		if let Some(indicator) = self.render_drop_indicator(window, cx) {
			root = root.child(indicator);
		}
		root
	}
}

impl Focusable for DockArea {
	fn focus_handle(&self, cx: &App) -> FocusHandle {
		let _ = cx;
		self.focus_handle.clone()
	}
}

impl EventEmitter<DockEvent> for DockArea {}

/// A registry that declines to key any panel; [`DockArea::save_state`] uses
/// it to produce an empty (but versioned) snapshot when no registry is set.
struct NoopPanelRegistry;

impl PanelRegistry for NoopPanelRegistry {
	fn panel_key(&self, _id: PanelId) -> Option<String> {
		None
	}

	fn build_panel(&self, _key: &str, _window: &mut Window, _cx: &mut App) -> Option<PanelHandle> {
		None
	}
}

impl DockArea {
	/// Re-writes every panel id in `node` from its interim (key-hashed) id to
	/// the rebuilt panel's real id, recording unmapped and duplicated ids for
	/// removal by the caller (their nodes collapse via `DockLayout::cleanup`).
	fn remap_node_ids(
		node: &mut DockNode,
		map: &HashMap<PanelId, PanelId>,
		used: &mut HashSet<PanelId>,
		to_remove: &mut Vec<PanelId>,
	) {
		match node {
			DockNode::Panel(id) => Self::remap_id(id, map, used, to_remove),
			DockNode::Tabs { panels, .. } => {
				for panel in panels.iter_mut() {
					Self::remap_id(panel, map, used, to_remove);
				}
			}
			DockNode::Split { children, .. } => {
				for child in children {
					Self::remap_node_ids(child, map, used, to_remove);
				}
			}
		}
	}

	fn remap_id(
		id: &mut PanelId,
		map: &HashMap<PanelId, PanelId>,
		used: &mut HashSet<PanelId>,
		to_remove: &mut Vec<PanelId>,
	) {
		let interim = *id;
		match map.get(&interim) {
			Some(&real) if used.insert(real) => *id = real,
			_ => to_remove.push(interim),
		}
	}

	/// Collects the paths of every node in `layout` (depth-first).
	fn collect_paths(&self, layout: &DockLayout, path: &NodePath, out: &mut HashSet<NodePath>) {
		let Some(root) = layout.root() else {
			return;
		};
		Self::collect_paths_in(root, path, out);
	}

	fn collect_paths_in(node: &DockNode, path: &NodePath, out: &mut HashSet<NodePath>) {
		out.insert(path.clone());
		if let DockNode::Split { children, .. } = node {
			for (index, child) in children.iter().enumerate() {
				let mut child_path = path.clone();
				child_path.0.push(index);
				Self::collect_paths_in(child, &child_path, out);
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{
		AnyElement, EventEmitter, Modifiers, MouseButton, Render, SharedString, TestAppContext,
		VisualTestContext, WindowHandle, dock::DockPanel, point, px, size,
	};
	use std::ops::Deref;

	/// A minimal dockable panel: a labeled box.
	struct TestPanel {
		id: u64,
		title: SharedString,
	}

	impl TestPanel {
		fn new(id: u64, title: &str) -> Self {
			Self {
				id,
				title: title.into(),
			}
		}
	}

	impl Render for TestPanel {
		fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
			div()
				.size_full()
				.flex()
				.items_center()
				.justify_center()
				.child(self.title.clone())
		}
	}

	impl EventEmitter<PanelEvent> for TestPanel {}

	impl DockPanel for TestPanel {
		fn panel_id(&self) -> PanelId {
			PanelId::new(self.id)
		}

		fn title(&self, _cx: &App) -> SharedString {
			self.title.clone()
		}

		fn tab_content(&self, _cx: &App) -> AnyElement {
			div().child(self.title.clone()).into_any_element()
		}
	}

	/// Root view hosting the dock area, so tests can reach the dock entity.
	struct DockHost {
		dock: Entity<DockArea>,
		/// The first panel, when the test needs to mutate its title behind
		/// the dock's cached snapshot (the language-refresh test).
		panel: Option<Entity<TestPanel>>,
	}

	impl Render for DockHost {
		fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
			div().size_full().child(self.dock.clone())
		}
	}

	/// Seeds a dock area with `panels` (the first becomes the root; the rest
	/// merge into it as tabs) plus the panels described by `targets` (added
	/// relative to an existing panel). Returns the typed window handle of the
	/// 800x600 test window.
	fn open_dock_window(
		test_app: &mut TestAppContext,
		panels: &[(u64, &str)],
		targets: &[(u64, u64, DropZone)],
	) -> WindowHandle<DockHost> {
		test_app.update(|cx| cx.init_colors());
		test_app.open_window(size(px(800.), px(600.)), |_window, cx| {
			let dock = cx.new(|cx| DockArea::new(cx));
			for (index, &(id, title)) in panels.iter().enumerate() {
				let panel = PanelHandle::new(cx.new(|_| TestPanel::new(id, title)), cx);
				dock.update(cx, |dock, cx| {
					let target = if index == 0 {
						None
					} else {
						Some(DropTarget {
							panel: Some(PanelId::new(panels[0].0)),
							zone: DropZone::Center,
						})
					};
					dock.add_panel(panel, target, cx);
				});
			}
			for &(id, target, zone) in targets {
				let panel = PanelHandle::new(cx.new(|_| TestPanel::new(id, "Extra")), cx);
				dock.update(cx, |dock, cx| {
					dock.add_panel(
						panel,
						Some(DropTarget {
							panel: Some(PanelId::new(target)),
							zone,
						}),
						cx,
					);
				});
			}
			DockHost { dock, panel: None }
		})
	}

	/// Describes a layout tree as a compact string for assertions:
	/// tabs are `[1,2]`, horizontal splits `a|b`, vertical splits `a/b`.
	fn describe(node: &DockNode) -> String {
		match node {
			DockNode::Split {
				direction,
				children,
				..
			} => {
				let sep = match direction {
					Axis::Horizontal => "|",
					Axis::Vertical => "/",
				};
				children
					.iter()
					.map(describe)
					.collect::<Vec<_>>()
					.join(sep)
			}
			DockNode::Tabs { panels, .. } => {
				let inner = panels
					.iter()
					.map(|p| p.raw().to_string())
					.collect::<Vec<_>>()
					.join(",");
				format!("[{inner}]")
			}
			DockNode::Panel(id) => id.raw().to_string(),
		}
	}

	/// Simulates a full tab drag: press at `from`, move past the drag threshold
	/// to `mid`, then continue to `to` and release.
	fn drag_tab(
		cx: &mut VisualTestContext,
		from: Point<Pixels>,
		mid: Point<Pixels>,
		to: Point<Pixels>,
	) {
		cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
		cx.simulate_mouse_move(mid, Some(MouseButton::Left), Modifiers::default());
		cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::default());
		cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
	}

	/// The layout of the dock area in the host root view, as a string.
	fn dock_layout(view: &Entity<DockHost>, cx: &mut VisualTestContext) -> String {
		cx.update(|_window, app| {
			describe(
				view.read(app)
					.dock
					.read(app)
					.layout()
					.root()
					.expect("dock has a root"),
			)
		})
	}

	/// The hovered drop target recorded during a drag, if any.
	fn hovered_target(view: &Entity<DockHost>, cx: &mut VisualTestContext) -> Option<DropTarget> {
		cx.update(|_window, app| {
			view.read(app)
				.dock
				.read(app)
				.drag
				.as_ref()
				.and_then(|d| d.hovered)
		})
	}

	/// The root split's ratios (the test layouts split at the root).
	fn root_ratios(view: &Entity<DockHost>, cx: &mut VisualTestContext) -> Vec<f32> {
		cx.update(|_window, app| {
			view.read(app)
				.dock
				.read(app)
				.layout()
				.split_ratios(&NodePath::default())
				.expect("root is a split")
		})
	}

	/// A UI language switch re-reads the panels' localized titles into the
	/// dock's cached snapshots (the tab strip renders those, and the panels
	/// are only re-rendered when notified).
	#[test]
	fn refresh_panel_titles_rereads_the_localized_titles() {
		let mut test_app = TestAppContext::single();
		test_app.update(|cx| cx.init_colors());
		let window = test_app.open_window(size(px(800.), px(600.)), |_window, cx| {
			let dock = cx.new(|cx| DockArea::new(cx));
			let panel = cx.new(|_| TestPanel::new(1, "Project"));
			let handle = PanelHandle::new(panel.clone(), cx);
			dock.update(cx, |dock, cx| dock.add_panel(handle, None, cx));
			DockHost {
				dock,
				panel: Some(panel),
			}
		});
		let view: Entity<DockHost> = window.root(&mut test_app).unwrap();
		let (dock, panel) = test_app.read(|app| {
			let host = view.read(app);
			(host.dock.clone(), host.panel.clone().expect("the test panel"))
		});

		// Registration snapshotted the title.
		let cached = |app: &App| {
			dock.read(app)
				.panel(PanelId::new(1))
				.expect("panel 1")
				.title()
				.clone()
		};
		assert_eq!(test_app.read(|app| cached(app)), SharedString::from("Project"));

		// The title changes (as a language switch would) behind the handle;
		// the refresh re-reads it.
		test_app.update(|app| {
			panel.update(app, |panel, _cx| panel.title = "项目".into());
			dock.update(app, |dock, cx| dock.refresh_panel_titles(cx));
		});
		assert_eq!(test_app.read(|app| cached(app)), SharedString::from("项目"));
	}

	/// Dragging a split's divider moves the boundary with the pointer: the
	/// grab never jumps the ratio to a clamp boundary, and each move applies
	/// only its own incremental delta to the current ratio (no compounding).
	#[test]
	fn dragging_a_split_handle_tracks_the_pointer() {
		let mut test_app = TestAppContext::single();
		let window = open_dock_window(&mut test_app, &[(1, "One")], &[(2, 1, DropZone::Right)]);
		let any_window = *window.deref();
		let view: Entity<DockHost> = window.root(&mut test_app).unwrap();
		let mut cx = VisualTestContext::from_window(any_window, &test_app).into_mut();

		assert_eq!(dock_layout(&view, cx), "1|2");

		// The divider sits at the middle of the 800px-wide window (a 6px
		// hitbox around x=400). Grab it, then drag to the right in two 50px
		// moves; the first drag-move only establishes the baseline.
		cx.simulate_mouse_down(point(px(400.), px(300.)), MouseButton::Left, Modifiers::default());
		cx.simulate_mouse_move(point(px(404.), px(300.)), Some(MouseButton::Left), Modifiers::default());
		cx.simulate_mouse_move(point(px(430.), px(300.)), Some(MouseButton::Left), Modifiers::default());
		cx.simulate_mouse_move(point(px(480.), px(300.)), Some(MouseButton::Left), Modifiers::default());

		// The left child's share grew by ~50px of the ~794px pair (800
		// minus the divider), not to a clamp boundary.
		let ratios = root_ratios(&view, &mut cx);
		assert!(
			(ratios[0] - (0.5 + 50.0 / 794.0)).abs() < 0.02,
			"ratio follows the pointer delta: {ratios:?}"
		);

		// A second move applies its own delta to the updated ratio; the
		// total drag distance is not re-applied on top.
		cx.simulate_mouse_move(point(px(530.), px(300.)), Some(MouseButton::Left), Modifiers::default());
		let ratios = root_ratios(&view, &mut cx);
		assert!(
			(ratios[0] - (0.5 + 100.0 / 794.0)).abs() < 0.02,
			"deltas accumulate incrementally: {ratios:?}"
		);

		cx.simulate_mouse_up(point(px(530.), px(300.)), MouseButton::Left, Modifiers::default());
	}

	/// Dragging a tab onto another panel's center merges it into that panel's
	/// tab group.
	#[test]
	fn drag_tab_merges_into_another_group() {
		let mut test_app = TestAppContext::single();
		let window = open_dock_window(&mut test_app, &[(1, "One"), (2, "Two")], &[(3, 1, DropZone::Right)]);
		let any_window = *window.deref();
		let view: Entity<DockHost> = window.root(&mut test_app).unwrap();
		let mut cx = VisualTestContext::from_window(any_window, &test_app).into_mut();

		// Layout: [1,2] | 3 (horizontal split, tab group on the left).
		assert_eq!(dock_layout(&view, cx), "[1,2]|3");

		// Drag tab 2 (left group, y=13 within the 26px strip) to the center of
		// panel 3 (right half of the 800x600 window).
		drag_tab(
			&mut cx,
			point(px(80.), px(13.)),
			point(px(126.), px(43.)),
			point(px(600.), px(300.)),
		);

		// Panel 2 now lives in a tab group with panel 3; the left group is
		// left with panel 1 alone.
		assert_eq!(dock_layout(&view, cx), "[1]|[3,2]");
	}

	/// Dragging a tab to the outer edge of the dock area splits the whole
	/// layout in that direction.
	#[test]
	fn drag_tab_to_root_edge_splits_layout() {
		let mut test_app = TestAppContext::single();
		let window = open_dock_window(&mut test_app, &[(1, "One"), (2, "Two")], &[]);
		let any_window = *window.deref();
		let view: Entity<DockHost> = window.root(&mut test_app).unwrap();
		let mut cx = VisualTestContext::from_window(any_window, &test_app).into_mut();

		assert_eq!(dock_layout(&view, cx), "[1,2]");

		// Drag tab 1 to the far right edge of the window (the edge band is a
		// quarter of the smaller dimension, so x=780 is within it).
		drag_tab(
			&mut cx,
			point(px(24.), px(13.)),
			point(px(60.), px(43.)),
			point(px(780.), px(300.)),
		);

		// The root split: [2] on the left, panel 1 alone on the right.
		assert_eq!(dock_layout(&view, cx), "[2]|1");
	}

	/// While dragging over a target, the hovered drop target is recorded so the
	/// drop indicator can render; a self-drop (own group) records none.
	#[test]
	fn drag_hover_records_target_but_not_self() {
		let mut test_app = TestAppContext::single();
		let window = open_dock_window(&mut test_app, &[(1, "One"), (2, "Two")], &[(3, 1, DropZone::Right)]);
		let any_window = *window.deref();
		let view: Entity<DockHost> = window.root(&mut test_app).unwrap();
		let mut cx = VisualTestContext::from_window(any_window, &test_app).into_mut();

		// Start a drag of tab 2 but stop before releasing.
		cx.simulate_mouse_down(point(px(80.), px(13.)), MouseButton::Left, Modifiers::default());
		cx.simulate_mouse_move(point(px(126.), px(43.)), Some(MouseButton::Left), Modifiers::default());
		cx.simulate_mouse_move(point(px(600.), px(300.)), Some(MouseButton::Left), Modifiers::default());

		// Over panel 3's center the target is recorded.
		assert_eq!(
			hovered_target(&view, &mut cx),
			Some(DropTarget {
				panel: Some(PanelId::new(3)),
				zone: DropZone::Center
			})
		);

		// Moving back over the dragged panel's own group clears the target.
		cx.simulate_mouse_move(point(px(200.), px(300.)), Some(MouseButton::Left), Modifiers::default());
		assert_eq!(hovered_target(&view, &mut cx), None);
	}

	/// Dragging a tab onto the edge band of another panel splits that panel in
	/// the corresponding direction instead of merging it as a tab.
	#[test]
	fn drag_tab_to_edge_of_other_panel_splits_it() {
		let mut test_app = TestAppContext::single();
		let window = open_dock_window(&mut test_app, &[(1, "One"), (2, "Two")], &[(3, 1, DropZone::Right)]);
		let any_window = *window.deref();
		let view: Entity<DockHost> = window.root(&mut test_app).unwrap();
		let mut cx = VisualTestContext::from_window(any_window, &test_app).into_mut();

		assert_eq!(dock_layout(&view, cx), "[1,2]|3");

		// Drop tab 2 just inside panel 3's left edge band (band is a quarter
		// of the smaller dimension: 100px here, so x=420 is within it).
		drag_tab(
			&mut cx,
			point(px(80.), px(13.)),
			point(px(126.), px(43.)),
			point(px(420.), px(300.)),
		);

		// Panel 2 lands as a new sibling to the left of panel 3 inside the
		// existing horizontal split.
		assert_eq!(dock_layout(&view, cx), "[1]|2|3");
	}

	/// Releasing a drag over the panel's own group is a no-op: the layout is
	/// left untouched.
	#[test]
	fn drag_release_over_own_group_is_noop() {
		let mut test_app = TestAppContext::single();
		let window = open_dock_window(&mut test_app, &[(1, "One"), (2, "Two")], &[]);
		let any_window = *window.deref();
		let view: Entity<DockHost> = window.root(&mut test_app).unwrap();
		let mut cx = VisualTestContext::from_window(any_window, &test_app).into_mut();

		assert_eq!(dock_layout(&view, cx), "[1,2]");

		// Drag tab 1 out of the strip, back over its own group's content, and
		// release there.
		drag_tab(
			&mut cx,
			point(px(24.), px(13.)),
			point(px(60.), px(43.)),
			point(px(200.), px(300.)),
		);

		assert_eq!(dock_layout(&view, cx), "[1,2]");
	}

	/// The tab close button renders with a real hitbox at the tab's right edge
	/// — not tucked directly after the title — so the ✕ affordance is visible
	/// and can't be mis-clicked.
	#[test]
	fn close_button_is_visible_and_right_aligned() {
		let mut test_app = TestAppContext::single();
		let window = open_dock_window(&mut test_app, &[(1, "One"), (2, "Two")], &[]);
		let any_window = *window.deref();
		let _view: Entity<DockHost> = window.root(&mut test_app).unwrap();
		let mut cx = VisualTestContext::from_window(any_window, &test_app).into_mut();
		cx.update(|window, cx| {
			window.draw(cx).clear();
		});

		let tab = cx.debug_bounds("dock-tab-1").expect("tab 1 rendered");
		let close = cx
			.debug_bounds("dock-tab-close-1")
			.expect("close button rendered for a closable tab");
		assert!(
			close.size.width > px(0.0) && close.size.height > px(0.0),
			"the close button has a usable hitbox: {close:?}"
		);
		// Right-aligned: the button's right edge sits inside the tab's own
		// padding (the tab is `px_2` = 8px), not after a short title. With a
		// short title and the old inline layout the button would float well
		// left of the tab's right edge.
		let gap = (tab.right() - close.right()).0;
		assert!(
			gap >= 0.0 && gap < 14.0,
			"close button pins to the tab's right edge (gap {gap}px; tab {tab:?}, close {close:?})"
		);
		assert!(
			close.left() >= tab.left() && close.right() <= tab.right() + px(1.0),
			"close button stays within the tab horizontally"
		);
	}

	/// Dragging a tab out of the dock and releasing over no drop target (here,
	/// beyond the window) tears the panel off into its own floating window.
	/// Closing that window re-docks the panel at its original position.
	#[test]
	fn dragging_a_tab_outside_the_dock_floats_it_and_closing_re_docks() {
		let mut test_app = TestAppContext::single();
		let window = open_dock_window(&mut test_app, &[(1, "One"), (2, "Two")], &[]);
		let any_window = *window.deref();
		let view: Entity<DockHost> = window.root(&mut test_app).unwrap();
		let mut cx = VisualTestContext::from_window(any_window, &test_app);

		assert_eq!(dock_layout(&view, &mut cx), "[1,2]");

		// Drag tab 1 to (900, 300) — past the 800x600 window — and release.
		// No drop target is hit, so the dock treats the release as a tear-off.
		drag_tab(
			&mut cx,
			point(px(24.), px(13.)),
			point(px(60.), px(43.)),
			point(px(900.), px(300.)),
		);
		cx.run_until_parked();
		// The next render observes the ended drag and floats the panel.
		cx.update(|window, cx| {
			window.draw(cx).clear();
		});
		cx.run_until_parked();

		// The panel left the dock; a second window now hosts it.
		assert_eq!(dock_layout(&view, &mut cx), "[2]", "panel 1 was floated");
		let floating = test_app
			.windows()
			.iter()
			.find_map(|handle| handle.downcast::<FloatingPanelWindow>())
			.expect("a floating window hosts the panel");
		let mut float_cx = VisualTestContext::from_window(*floating.deref(), &test_app);

		// Closing the floating window returns the panel to its original group.
		assert!(
			float_cx.simulate_close(),
			"closing the floating window is allowed"
		);
		assert_eq!(
			dock_layout(&view, &mut cx),
			"[2,1]",
			"the panel re-docked beside its original group"
		);
	}

	/// Closing a floating window for good (the 窗口 menu's toggle on a
	/// torn-off panel, see [`DockArea::close_floating`]) removes the panel
	/// without re-docking it.
	#[test]
	fn close_floating_removes_the_panel_without_redocking() {
		let mut test_app = TestAppContext::single();
		let window = open_dock_window(&mut test_app, &[(1, "One"), (2, "Two")], &[]);
		let any_window = *window.deref();
		let view: Entity<DockHost> = window.root(&mut test_app).unwrap();
		let mut cx = VisualTestContext::from_window(any_window, &test_app);

		// Tear panel 1 off first.
		drag_tab(
			&mut cx,
			point(px(24.), px(13.)),
			point(px(60.), px(43.)),
			point(px(900.), px(300.)),
		);
		cx.run_until_parked();
		cx.update(|window, cx| {
			window.draw(cx).clear();
		});
		cx.run_until_parked();
		assert_eq!(dock_layout(&view, &mut cx), "[2]");

		// Explicitly close the floating window: the panel stays gone.
		let dock = cx.read(|app| view.read(app).dock.clone());
		dock.update(&mut cx, |dock, cx| dock.close_floating(PanelId::new(1), cx));
		cx.run_until_parked();
		assert_eq!(
			dock_layout(&view, &mut cx),
			"[2]",
			"an explicit close does not re-dock the panel"
		);
	}
}
