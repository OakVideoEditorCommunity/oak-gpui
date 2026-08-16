//! The [`DockArea`] view: hosts panels, renders the layout tree, and handles
//! drag-to-dock interaction.

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
	SharedString, Stateful, Styled, Subscription, Window, deferred, div, hsla, px, relative, size,
};
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

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
	/// One tab-strip entity per `Tabs` node, keyed by the node's current
	/// path. Re-created when the tree changes shape and pruned each render.
	/// The subscription keeps the strip's events routed back to this view.
	tab_bars: HashMap<NodePath, (Entity<TabBar>, Subscription)>,
	/// One split-handle entity per boundary of each `Split` node, keyed by
	/// the node's current path and the boundary index. Holds transient drag
	/// state (`SplitHandle::drag_origin`) across frames; pruned with the tab
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
	/// **Deferred**: floating panels depend on unverified multi-window
	/// capabilities; see the [`floating`](crate::dock::FloatingPanelWindow)
	/// docs. When implemented, this removes the panel from the layout (like
	/// [`remove_panel`](DockArea::remove_panel) but without emitting
	/// [`DockEvent::PanelRemoved`]) and opens a
	/// [`FloatingPanelWindow`](crate::dock::FloatingPanelWindow) hosting it;
	/// dropping the window back over a dock area re-docks the panel. Until
	/// then, always returns `false`.
	///
	/// Returns `true` if the panel was floated.
	pub fn float_panel(
		&mut self,
		_id: PanelId,
		_window: &mut Window,
		_cx: &mut Context<Self>,
	) -> bool {
		false
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

	/// Emits [`DockEvent::LayoutChanged`] and repaints.
	fn emit_layout_changed(&mut self, cx: &mut Context<Self>) {
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
		let pair_extent = full_extent * pair_total / total.max(1.0);
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
				let mut container = div()
					.flex()
					.size_full()
					.overflow_hidden()
					.id(ElementId::named_usize("dock-split", path_key(path)))
					.on_drag_move::<SplitHandleDrag>(cx.listener(
						move |this, event: &DragMoveEvent<SplitHandleDrag>, _window, cx| {
							let drag = event.drag(cx);
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
		// A drag that ended without a drop (released outside the dock area)
		// leaves transient drag state behind; clear it on the next render.
		if self.drag.is_some() && !cx.has_active_drag() {
			self.drag = None;
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
			DockHost { dock }
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
}
