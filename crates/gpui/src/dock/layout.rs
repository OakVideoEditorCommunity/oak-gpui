//! The dock layout tree, drop targeting, and serializable layout snapshots.
//!
//! [`DockLayout`] owns the *shape* of a docked workspace as a tree of
//! [`DockNode`]s. The tree stores only [`PanelId`]s; the actual panel views
//! live in the [`DockArea`](crate::dock::DockArea)(crate::dock::DockArea). All structural edits go
//! through the layout operations on `DockLayout` ([`insert_panel`],
//! [`remove_panel`], [`move_panel`], [`resize_split`]), which maintain the
//! invariants documented on [`DockNode`]; [`cleanup`](DockLayout::cleanup)
//! re-normalizes after edits.
//!
//! Persistence is split in two: [`DockLayoutState`](crate::dock::DockLayoutState) is a serde snapshot of the
//! tree with panels referenced by *string keys*, and [`PanelRegistry`] is the
//! application-implemented bridge between keys and live panels.

use crate::Axis;
use crate::dock::{PanelHandle, PanelId};
use crate::{App, Window};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};

/// One node of the dock layout tree.
///
/// # Invariants
///
/// These are maintained by every [`DockLayout`] mutator (and re-established
/// by [`DockLayout::cleanup`]); code constructing nodes by hand must uphold
/// them:
///
/// - `Split.children` has at least two entries and contains no direct
///   `Split` child with the same `direction` (such nests are flattened).
/// - `Split.ratio` is finite and clamped to `(0.0, 1.0)` exclusive; see
///   [`DockLayout::resize_split`] for clamping against minimum panel sizes.
/// - `Tabs.panels` is non-empty and `Tabs.active < panels.len()`.
/// - Every [`PanelId`] occurs at most once in the whole tree.
///
/// A tree consisting of a single `Panel` leaf is valid. An "empty" layout is
/// represented by [`DockLayout::root`] being `None`, never by empty nodes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DockNode {
    /// A row or column of child nodes sized proportionally.
    Split {
        /// The axis along which children are laid out: [`Axis::Horizontal`]
        /// places children side by side, [`Axis::Vertical`] stacks them.
        direction: Axis,
        /// Fraction of the available extent (along `direction`) assigned to
        /// the first child, relative to the remaining children. For two
        /// children this is simply the first child's share. Adjusted by
        /// dragging a split handle; see
        /// [`DockLayout::resize_split`].
        ratio: f32,
        /// The children, in layout order. Never empty, never a single child,
        /// and never contains a nested `Split` with the same `direction`.
        children: Vec<DockNode>,
    },
    /// A tab group showing one of several panels at a time.
    Tabs {
        /// Panels in tab order. Non-empty.
        panels: Vec<PanelId>,
        /// Index into `panels` of the visible tab. Always `< panels.len()`.
        active: usize,
    },
    /// A leaf holding exactly one panel.
    Panel(PanelId),
}

/// Where, relative to a drop target, a dragged panel should be docked.
///
/// Every tab group / leaf panel offers all five zones while a drag is in
/// flight; the root additionally offers its four outer edges (see
/// [`DropTarget`]). Hit-testing from a cursor position is done by
/// [`DockArea::drop_zone_at`](crate::dock::DockArea::drop_zone_at).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DropZone {
    /// Dock to the left of the target: split the target's node horizontally,
    /// inserting the dragged panel as the new left child.
    Left,
    /// Dock to the right of the target (horizontal split, new right child).
    Right,
    /// Dock above the target (vertical split, new top child).
    Top,
    /// Dock below the target (vertical split, new bottom child).
    Bottom,
    /// Merge the dragged panel into the target as a new tab. The dragged
    /// panel becomes the active tab of the resulting group.
    Center,
}

impl DropZone {
    /// All five zones, in declaration order. Useful for painting affordances.
    pub const ALL: [DropZone; 5] = [
        DropZone::Left,
        DropZone::Right,
        DropZone::Top,
        DropZone::Bottom,
        DropZone::Center,
    ];

    /// Returns `true` if this zone splits the target rather than merging into
    /// it as a tab — i.e. anything except [`DropZone::Center`].
    pub const fn is_split(self) -> bool {
        !matches!(self, DropZone::Center)
    }

    /// Returns `true` if this zone merges the dragged panel into the target
    /// as a tab ([`DropZone::Center`]).
    pub const fn is_merge(self) -> bool {
        matches!(self, DropZone::Center)
    }

    /// Returns the split axis this zone implies, or `None` for
    /// [`DropZone::Center`]. `Left`/`Right` split along
    /// [`Axis::Horizontal`], `Top`/`Bottom` along [`Axis::Vertical`].
    pub const fn split_axis(self) -> Option<Axis> {
        match self {
            DropZone::Left | DropZone::Right => Some(Axis::Horizontal),
            DropZone::Top | DropZone::Bottom => Some(Axis::Vertical),
            DropZone::Center => None,
        }
    }
}

/// A concrete place a dragged panel can be dropped.
///
/// Combines the panel (tab group) being hovered with the zone within it.
/// When `panel` is `None`, the target is an outer edge of the whole layout
/// root — this is how a drop on the window's very edge splits the entire
/// tree. `zone` must be an edge zone (never [`DropZone::Center`]) when
/// `panel` is `None`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DropTarget {
    /// The panel whose tab group / leaf is targeted, or `None` to target an
    /// outer edge of the root.
    pub panel: Option<PanelId>,
    /// The zone within the target.
    pub zone: DropZone,
}

/// Path addressing a node within a [`DockLayout`]: child indices from the root.
///
/// `NodePath(vec![])` addresses the root; each successive index descends into
/// that node's `children` (for splits) or is invalid (for `Tabs`/`Panel`,
/// which have no node children). Paths are invalidated by any structural edit
/// and must be re-derived with [`DockLayout::find_panel`] afterwards.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodePath(pub Vec<usize>);

/// Returns a stable `usize` key for `path`, used to derive element ids for
/// per-node chrome (split containers, split handles).
///
/// Two different paths always produce different keys; equal paths produce
/// equal keys.
pub(crate) fn path_key(path: &NodePath) -> usize {
    let mut hasher = DefaultHasher::new();
    path.0.hash(&mut hasher);
    hasher.finish() as usize
}

/// The dock layout tree: the shape of one [`DockArea`](crate::dock::DockArea)'s workspace.
///
/// The tree is pure data (no views, no GPUI handles) and is cheap to clone.
/// All edits go through the methods below so the [`DockNode`] invariants are
/// preserved; after any sequence of edits the mutators run
/// [`cleanup`](DockLayout::cleanup) themselves.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DockLayout {
    root: Option<DockNode>,
}

impl DockLayout {
    /// Creates an empty layout (no root node).
    pub fn new() -> Self {
        Self { root: None }
    }

    /// Returns the root node, or `None` if the layout is empty.
    pub fn root(&self) -> Option<&DockNode> {
        self.root.as_ref()
    }

    /// Returns the root node mutably, or `None` if the layout is empty.
    pub(crate) fn root_mut(&mut self) -> Option<&mut DockNode> {
        self.root.as_mut()
    }

    /// Returns `true` if `panel` occurs anywhere in the tree.
    pub fn contains(&self, panel: PanelId) -> bool {
        let Some(root) = self.root.as_ref() else {
            return false;
        };
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            match node {
                DockNode::Panel(id) => {
                    if *id == panel {
                        return true;
                    }
                }
                DockNode::Tabs { panels, .. } => {
                    if panels.contains(&panel) {
                        return true;
                    }
                }
                DockNode::Split { children, .. } => stack.extend(children),
            }
        }
        false
    }

    /// Returns the path to the node containing `panel`, or `None` if the
    /// panel is not in the tree.
    ///
    /// For a `Panel` leaf the path addresses the leaf itself; for a tab in a
    /// `Tabs` group it addresses the `Tabs` node (inspect the node to get the
    /// tab index).
    pub fn find_panel(&self, panel: PanelId) -> Option<NodePath> {
        let root = self.root.as_ref()?;
        Self::find_panel_in(root, panel, &mut Vec::new())
    }

    /// Depth-first search recording the path taken; the path of the node
    /// containing `panel` when found.
    fn find_panel_in(node: &DockNode, panel: PanelId, path: &mut Vec<usize>) -> Option<NodePath> {
        match node {
            DockNode::Panel(id) if *id == panel => Some(NodePath(path.clone())),
            DockNode::Panel(_) => None,
            DockNode::Tabs { panels, .. } => {
                if panels.contains(&panel) {
                    Some(NodePath(path.clone()))
                } else {
                    None
                }
            }
            DockNode::Split { children, .. } => {
                for (index, child) in children.iter().enumerate() {
                    path.push(index);
                    if let Some(found) = Self::find_panel_in(child, panel, path) {
                        return Some(found);
                    }
                    path.pop();
                }
                None
            }
        }
    }

    /// Inserts `panel` at `target`.
    ///
    /// - `target: None` — the layout must be empty; `panel` becomes the root.
    ///   If the layout is *not* empty this is a no-op returning `false`
    ///   (choose a concrete [`DropTarget`] instead).
    /// - Edge zones — wraps/splits the target node along
    ///   [`DropZone::split_axis`]. If the target already sits inside a split
    ///   with the same axis, the panel is inserted as a sibling instead of
    ///   nesting.
    /// - [`DropZone::Center`] — appends `panel` to the target's tab group
    ///   (converting a `Panel` leaf into `Tabs`) and makes it active.
    ///
    /// Returns `true` if the tree changed. Fails (returns `false`) if
    /// `panel` is already present — use [`move_panel`](DockLayout::move_panel)
    /// to relocate — or if the target panel no longer exists.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if `target.panel` is `None` and
    /// `target.zone` is [`DropZone::Center`].
    pub fn insert_panel(&mut self, panel: PanelId, target: Option<DropTarget>) -> bool {
        if self.contains(panel) {
            return false;
        }

        let Some(target) = target else {
            if self.root.is_some() {
                return false;
            }
            self.root = Some(DockNode::Panel(panel));
            return true;
        };

        debug_assert!(
            target.panel.is_some() || target.zone != DropZone::Center,
            "a root-edge target must use an edge zone, never Center"
        );

        let Some(target_panel) = target.panel else {
            // An outer edge of the whole tree: split the root itself.
            let changed = self.insert_at_root_edge(panel, target.zone);
            if changed {
                self.cleanup();
            }
            return changed;
        };

        if !self.contains(target_panel) {
            return false;
        }

        if target.zone.is_merge() {
            self.insert_into_tabs(panel, target_panel);
        } else {
            self.insert_at_edge(panel, target_panel, target.zone);
        }
        self.cleanup();
        true
    }

    /// Inserts `panel` at an outer edge of the root, splitting the whole tree
    /// unless the root is already a split along the same axis (in which case
    /// the panel becomes a sibling).
    fn insert_at_root_edge(&mut self, panel: PanelId, zone: DropZone) -> bool {
        let Some(mut root) = self.root.take() else {
            return false;
        };
        let axis = zone
            .split_axis()
            .expect("root-edge zones always imply a split axis");
        let before = matches!(zone, DropZone::Left | DropZone::Top);

        if let DockNode::Split {
            direction,
            children,
            ..
        } = &mut root
        {
            if *direction == axis {
                if before {
                    children.insert(0, DockNode::Panel(panel));
                } else {
                    children.push(DockNode::Panel(panel));
                }
                self.root = Some(root);
                return true;
            }
        }

        self.root = Some(DockNode::Split {
            direction: axis,
            ratio: 0.5,
            children: if before {
                vec![DockNode::Panel(panel), root]
            } else {
                vec![root, DockNode::Panel(panel)]
            },
        });
        true
    }

    /// Inserts `panel` relative to the node containing `target_panel`,
    /// splitting it unless it already sits in a same-axis split (then the
    /// panel is inserted as a sibling instead).
    fn insert_at_edge(&mut self, panel: PanelId, target_panel: PanelId, zone: DropZone) {
        let axis = zone.split_axis().expect("edge zones imply a split axis");
        let path = self
            .find_panel(target_panel)
            .expect("caller checked the target exists");
        let before = matches!(zone, DropZone::Left | DropZone::Top);

        // If the target sits inside a split along the same axis, insert the
        // panel as a sibling rather than nesting a split inside a split.
        let mut parent = path.0.clone();
        if parent.pop().is_some() {
            let parent_path = NodePath(parent);
            if let Some(DockNode::Split {
                direction,
                children,
                ..
            }) = self.node_at_mut(&parent_path)
            {
                if *direction == axis {
                    let index = *path.0.last().expect("non-root path has a last index");
                    children.insert(
                        if before { index } else { index + 1 },
                        DockNode::Panel(panel),
                    );
                    return;
                }
            }
        }

        // Otherwise wrap the target node in a fresh split.
        let old_node = self
            .node_at(&path)
            .expect("path was just derived")
            .clone();
        let new_node = DockNode::Panel(panel);
        let replacement = DockNode::Split {
            direction: axis,
            ratio: 0.5,
            children: if before {
                vec![new_node, old_node]
            } else {
                vec![old_node, new_node]
            },
        };
        *self
            .node_at_mut(&path)
            .expect("path was just derived") = replacement;
    }

    /// Appends `panel` to the tab group containing `target_panel` (converting
    /// a `Panel` leaf into a `Tabs` node) and makes it active.
    fn insert_into_tabs(&mut self, panel: PanelId, target_panel: PanelId) {
        let path = self
            .find_panel(target_panel)
            .expect("caller checked the target exists");
        let node = self.node_at_mut(&path).expect("path was just derived");
        match node {
            DockNode::Panel(_) => {
                *node = DockNode::Tabs {
                    panels: vec![target_panel, panel],
                    active: 1,
                };
            }
            DockNode::Tabs { panels, active } => {
                panels.push(panel);
                *active = panels.len() - 1;
            }
            DockNode::Split { .. } => unreachable!("find_panel never addresses a Split"),
        }
    }

    /// Removes `panel` from the tree, running [`cleanup`](DockLayout::cleanup)
    /// to collapse nodes left empty or single-childed.
    ///
    /// Returns `true` if the panel was present. Removing the last panel sets
    /// the root to `None`.
    pub fn remove_panel(&mut self, panel: PanelId) -> bool {
        let Some(root) = self.root.as_mut() else {
            return false;
        };
        if !Self::remove_from_node(root, panel) {
            return false;
        }
        self.cleanup();
        true
    }

    /// Recursively removes `panel` from `node`, returning whether it was found.
    fn remove_from_node(node: &mut DockNode, panel: PanelId) -> bool {
        match node {
            DockNode::Panel(id) => *id == panel,
            DockNode::Tabs { panels, active } => {
                let Some(index) = panels.iter().position(|&other| other == panel) else {
                    return false;
                };
                panels.remove(index);
                if !panels.is_empty() {
                    *active = (*active).min(panels.len() - 1);
                }
                true
            }
            DockNode::Split { children, .. } => children
                .iter_mut()
                .any(|child| Self::remove_from_node(child, panel)),
        }
    }

    /// Atomically moves `panel` to `target`.
    ///
    /// Equivalent to [`remove_panel`](DockLayout::remove_panel) followed by
    /// [`insert_panel`](DockLayout::insert_panel), but a no-op (returning
    /// `false`) if `panel` is not present or the drop would land the panel
    /// back in its own position (e.g. `Center` onto its own group). Moving
    /// the only panel of a group away collapses the group.
    pub fn move_panel(&mut self, panel: PanelId, target: DropTarget) -> bool {
        if !self.contains(panel) {
            return false;
        }
        // Dropping onto the panel's own node — its group for `Center`, its own
        // node for edge zones — would target a node that no longer exists
        // after the removal below, so treat every self-drop as a no-op.
        if target.panel == Some(panel) {
            return false;
        }
        let removed = self.remove_panel(panel);
        debug_assert!(removed, "panel presence was checked above");
        let inserted = self.insert_panel(panel, Some(target));
        debug_assert!(inserted, "drop target must remain valid after the removal");
        inserted
    }

    /// Sets the `ratio` of the `Split` node at `path`.
    ///
    /// `ratio` is clamped so that no child shrinks below its minimum extent
    /// (see the min-size constants in
    /// the `split_handle` module); out-of-range values are
    /// clamped rather than rejected.
    ///
    /// # Panics
    ///
    /// Panics if `path` does not address a [`DockNode::Split`].
    pub fn resize_split(&mut self, path: &NodePath, ratio: f32) {
        let Some(node) = self.node_at_mut(path) else {
            panic!("resize_split: path {path:?} does not address a node");
        };
        let DockNode::Split { ratio: current, .. } = node else {
            panic!("resize_split: path {path:?} does not address a Split node");
        };
        // Pixel-level minimum extents (see split_handle::MIN_CHILD_EXTENT)
        // depend on the rendered size and cannot be enforced on a ratio;
        // clamp to a conservative fraction so both children keep a share.
        *current = ratio.clamp(0.05, 0.95);
    }

    /// Re-establishes the [`DockNode`] invariants after structural edits.
    ///
    /// Concretely: removes empty `Tabs` nodes, replaces single-child `Split`s
    /// with their child, flattens same-direction `Split` nests, and clamps
    /// `active` tab indices into range. All public mutators call this
    /// internally; call it manually only after mutating nodes obtained via
    /// interior references (which the API avoids exposing for this reason).
    pub fn cleanup(&mut self) {
        self.root = self.root.take().and_then(Self::cleanup_node);
    }

    /// Normalizes a single node, returning `None` when it collapses away.
    fn cleanup_node(node: DockNode) -> Option<DockNode> {
        match node {
            DockNode::Panel(_) => Some(node),
            DockNode::Tabs {
                mut panels,
                mut active,
            } => {
                if panels.is_empty() {
                    return None;
                }
                active = active.min(panels.len() - 1);
                Some(DockNode::Tabs { panels, active })
            }
            DockNode::Split {
                direction,
                ratio,
                children,
            } => {
                let children: Vec<DockNode> = children
                    .into_iter()
                    .filter_map(Self::cleanup_node)
                    .collect();
                // Flatten direct same-direction split nests.
                let mut flat = Vec::with_capacity(children.len());
                for child in children {
                    match child {
                        DockNode::Split {
                            direction: nested_direction,
                            children: nested_children,
                            ..
                        } if nested_direction == direction => flat.extend(nested_children),
                        other => flat.push(other),
                    }
                }
                let ratio = if ratio.is_finite() {
                    ratio.clamp(0.05, 0.95)
                } else {
                    0.5
                };
                match flat.len() {
                    0 => None,
                    1 => Some(flat.pop().expect("len == 1")),
                    _ => Some(DockNode::Split {
                        direction,
                        ratio,
                        children: flat,
                    }),
                }
            }
        }
    }

    /// Visits every [`PanelId`] in the tree, in depth-first order.
    pub fn panels(&self) -> Vec<PanelId> {
        let mut panels = Vec::new();
        if let Some(root) = &self.root {
            Self::collect_panels(root, &mut panels);
        }
        panels
    }

    fn collect_panels(node: &DockNode, out: &mut Vec<PanelId>) {
        match node {
            DockNode::Panel(id) => out.push(*id),
            DockNode::Tabs { panels, .. } => out.extend_from_slice(panels),
            DockNode::Split { children, .. } => {
                for child in children {
                    Self::collect_panels(child, out);
                }
            }
        }
    }

    /// Returns the `ratio` of the `Split` node at `path`, or `None` if `path`
    /// does not address a split.
    pub fn split_ratio(&self, path: &NodePath) -> Option<f32> {
        match self.node_at(path) {
            Some(DockNode::Split { ratio, .. }) => Some(*ratio),
            _ => None,
        }
    }

    /// Makes `panel` the active tab of the `Tabs` node at `path`, if present.
    pub fn set_tabs_active(&mut self, path: &NodePath, panel: PanelId) -> bool {
        let Some(node) = self.node_at_mut(path) else {
            return false;
        };
        let DockNode::Tabs { panels, active } = node else {
            return false;
        };
        match panels.iter().position(|&other| other == panel) {
            Some(index) => {
                *active = index;
                true
            }
            None => false,
        }
    }

    /// Returns the node at `path`, or `None` if the path is invalid.
    fn node_at(&self, path: &NodePath) -> Option<&DockNode> {
        let mut node = self.root.as_ref()?;
        for &index in &path.0 {
            let DockNode::Split { children, .. } = node else {
                return None;
            };
            node = children.get(index)?;
        }
        Some(node)
    }

    /// Returns a mutable reference to the node at `path`, or `None`.
    pub(crate) fn node_at_mut(&mut self, path: &NodePath) -> Option<&mut DockNode> {
        let mut node = self.root.as_mut()?;
        for &index in &path.0 {
            let DockNode::Split { children, .. } = node else {
                return None;
            };
            node = children.get_mut(index)?;
        }
        Some(node)
    }
}

/// Application-provided bridge between panel string keys and live panels.
///
/// Panels are live views and cannot be serialized, so persistence stores only
/// a stable string key per panel. On save, [`panel_key`](PanelRegistry::panel_key)
/// maps each [`PanelId`] to its key; on restore,
/// [`build_panel`](PanelRegistry::build_panel) reconstructs a fresh view from
/// a key. The application owns the mapping — e.g. `"media-bin"`,
/// `"program-monitor"`, or per-project keys like `"inspector:clip-42"`.
///
/// Keys must round-trip: a key produced by `panel_key` must be accepted by
/// `build_panel`. Keys that fail to rebuild are dropped from the restored
/// layout (with their positions collapsed by [`DockLayout::cleanup`]), so a
/// panel type removed in a newer app version degrades gracefully instead of
/// failing the whole restore.
///
/// # Examples
///
/// ```ignore
/// struct OakPanelRegistry;
///
/// impl PanelRegistry for OakPanelRegistry {
///     fn panel_key(&self, id: PanelId) -> Option<String> {
///         match id.raw() {
///             1 => Some("media-bin".into()),
///             2 => Some("program-monitor".into()),
///             _ => None,
///         }
///     }
///
///     fn build_panel(&self, key: &str, window: &mut Window, cx: &mut App)
///         -> Option<PanelHandle>
///     {
///         match key {
///             "media-bin" => Some(PanelHandle::new(cx.new(|_| MediaBin::new()), cx)),
///             "program-monitor" => Some(PanelHandle::new(cx.new(|_| Monitor::new()), cx)),
///             _ => None,
///         }
///     }
/// }
/// ```
pub trait PanelRegistry: 'static {
    /// Returns the stable string key for a live panel, or `None` if the panel
    /// is transient and should be omitted from saved layouts.
    fn panel_key(&self, id: PanelId) -> Option<String>;

    /// Rebuilds the panel identified by `key`, or returns `None` if the key
    /// is unknown (the panel is then skipped during restore).
    ///
    /// Called on the main thread during
    /// [`DockArea::restore_state`](crate::dock::DockArea::restore_state)(crate::dock::DockArea::restore_state); the registry may create entities with
    /// `cx.new` and perform per-panel setup, but should not open windows or
    /// otherwise mutate the dock area.
    fn build_panel(&self, key: &str, window: &mut Window, cx: &mut App) -> Option<PanelHandle>;
}

/// A serde-serializable snapshot of a [`DockLayout`], with panels referenced
/// by registry string keys.
///
/// This is the type to persist (via `serde_json`, a settings file, ...).
/// Capture it from a live layout with [`DockLayoutState::capture`] and turn it
/// back into a tree with [`DockLayoutState::to_layout`]; then hand it to
/// [`DockArea::restore_state`](crate::dock::DockArea::restore_state)(crate::dock::DockArea::restore_state), which rebuilds the views through the
/// [`PanelRegistry`].
///
/// The serialized form is versioned via the `version` field; the current
/// version is [`DockLayoutState::VERSION`]. Unknown/newer versions should be
/// rejected by the caller before restoring.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DockLayoutState {
    /// Snapshot format version; written by [`capture`](DockLayoutState::capture),
    /// checked by the caller on load.
    pub version: u32,
    root: Option<SerializedNode>,
    /// Registry keys for panels that existed when the snapshot was taken but
    /// whose `panel_key` returned `Some` while they were not reachable in the
    /// tree (reserved for floating panels; see
    /// [`FloatingPanelWindow`](crate::dock::FloatingPanelWindow)). Empty until floating support
    /// lands.
    #[serde(default)]
    pub floating: BTreeMap<String, SerializedFloating>,
}

/// Reserved per-floating-panel data inside [`DockLayoutState`](crate::dock::DockLayoutState).
///
/// Placeholder for the deferred floating-window feature (see
/// [`FloatingPanelWindow`](crate::dock::FloatingPanelWindow)): remembers that a panel was undocked
/// and where its window was. Not yet produced by
/// [`DockLayoutState::capture`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SerializedFloating {
    /// Logical x position of the floating window, in pixels.
    pub x: f32,
    /// Logical y position of the floating window, in pixels.
    pub y: f32,
    /// Width of the floating window, in pixels.
    pub width: f32,
    /// Height of the floating window, in pixels.
    pub height: f32,
}

/// A serialized [`DockNode`] with panel keys instead of ids.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum SerializedNode {
    /// Serialized [`DockNode::Split`].
    Split {
        /// See [`DockNode::Split::direction`].
        direction: Axis,
        /// See [`DockNode::Split::ratio`].
        ratio: f32,
        /// See [`DockNode::Split::children`].
        children: Vec<SerializedNode>,
    },
    /// Serialized [`DockNode::Tabs`]; `active` is an index into `panels`.
    Tabs {
        /// Registry keys of the tabbed panels, in tab order.
        panels: Vec<String>,
        /// Active tab index.
        active: usize,
    },
    /// Serialized [`DockNode::Panel`], holding the panel's registry key.
    Panel(String),
}

impl DockLayoutState {
    /// The snapshot format version written by
    /// [`capture`](DockLayoutState::capture).
    pub const VERSION: u32 = 1;

    /// Snapshots `layout`, translating panel ids to string keys via
    /// `registry`.
    ///
    /// Panels whose [`PanelRegistry::panel_key`] returns `None` are omitted
    /// from the snapshot; their nodes are collapsed as if removed.
    pub fn capture(layout: &DockLayout, registry: &dyn PanelRegistry) -> Self {
        let root = layout
            .root
            .as_ref()
            .and_then(|node| Self::serialize_node(node, registry));
        Self {
            version: Self::VERSION,
            root,
            floating: BTreeMap::new(),
        }
    }

    /// Serializes `node`, dropping panels without a key; `None` when the node
    /// collapses away entirely.
    fn serialize_node(node: &DockNode, registry: &dyn PanelRegistry) -> Option<SerializedNode> {
        match node {
            DockNode::Panel(id) => registry.panel_key(*id).map(SerializedNode::Panel),
            DockNode::Tabs { panels, active } => {
                let active_panel = panels.get(*active).copied();
                let mut serialized = Vec::new();
                let mut serialized_active = 0;
                for id in panels.iter() {
                    if let Some(key) = registry.panel_key(*id) {
                        if Some(*id) == active_panel {
                            serialized_active = serialized.len();
                        }
                        serialized.push(key);
                    }
                }
                if serialized.is_empty() {
                    return None;
                }
                serialized_active = serialized_active.min(serialized.len() - 1);
                Some(SerializedNode::Tabs {
                    panels: serialized,
                    active: serialized_active,
                })
            }
            DockNode::Split {
                direction,
                ratio,
                children,
            } => {
                let children: Vec<SerializedNode> = children
                    .iter()
                    .filter_map(|child| Self::serialize_node(child, registry))
                    .collect();
                match children.len() {
                    0 => None,
                    1 => children.into_iter().next(),
                    _ => Some(SerializedNode::Split {
                        direction: *direction,
                        ratio: *ratio,
                        children,
                    }),
                }
            }
        }
    }

    /// Rebuilds the pure tree shape, leaving view reconstruction to the
    /// caller (see [`DockArea::restore_state`](crate::dock::DockArea::restore_state)(crate::dock::DockArea::restore_state), which resolves keys through
    /// the registry).
    ///
    /// The returned layout is normalized ([`DockLayout::cleanup`] has run).
    pub fn to_layout(&self) -> DockLayout {
        let root = self.root.as_ref().and_then(Self::deserialize_node);
        let mut layout = DockLayout { root };
        layout.cleanup();
        layout
    }

    /// Deserializes a single node, addressing panels by deterministic interim
    /// ids (see [`interim_id`]); `None` when the node collapses away.
    fn deserialize_node(node: &SerializedNode) -> Option<DockNode> {
        match node {
            SerializedNode::Panel(key) => Some(DockNode::Panel(interim_id(key))),
            SerializedNode::Tabs { panels, active } => {
                if panels.is_empty() {
                    return None;
                }
                let panels: Vec<PanelId> = panels.iter().map(|key| interim_id(key)).collect();
                Some(DockNode::Tabs {
                    active: (*active).min(panels.len() - 1),
                    panels,
                })
            }
            SerializedNode::Split {
                direction,
                ratio,
                children,
            } => {
                let children: Vec<DockNode> = children
                    .iter()
                    .filter_map(Self::deserialize_node)
                    .collect();
                match children.len() {
                    0 => None,
                    1 => children.into_iter().next(),
                    _ => Some(DockNode::Split {
                        direction: *direction,
                        ratio: *ratio,
                        children,
                    }),
                }
            }
        }
    }

    /// Collects the registry keys referenced by this snapshot, depth-first.
    ///
    /// Used by [`DockArea::restore_state`](crate::dock::DockArea::restore_state)(crate::dock::DockArea::restore_state) to rebuild panels
    /// (and learn their real ids) before re-keying the tree.
    pub(crate) fn keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        if let Some(root) = &self.root {
            Self::collect_keys(root, &mut keys);
        }
        keys
    }

    fn collect_keys(node: &SerializedNode, out: &mut Vec<String>) {
        match node {
            SerializedNode::Panel(key) => out.push(key.clone()),
            SerializedNode::Tabs { panels, .. } => out.extend(panels.iter().cloned()),
            SerializedNode::Split { children, .. } => {
                for child in children {
                    Self::collect_keys(child, out);
                }
            }
        }
    }
}

/// Maps a registry key to a deterministic interim [`PanelId`].
///
/// [`DockLayoutState::to_layout`] cannot know the real ids of rebuilt panels,
/// so it addresses nodes with ids derived from the key. `restore_state` later
/// rebuilds the real panels and re-maps the tree using the same hash, so the
/// two passes agree.
pub(crate) fn interim_id(key: &str) -> PanelId {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    PanelId::new(hasher.finish())
}
