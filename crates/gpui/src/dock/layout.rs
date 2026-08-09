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
/// - `Split.ratios` has exactly one entry per child, each strictly positive,
///   and the entries sum to `1.0` (they are the fraction of the parent extent
///   given to each child; see [`DockLayout::resize_split_child`]).
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
        /// The fraction of the available extent (along `direction`) assigned
        /// to each child, in child order. Entries sum to `1.0`, so a split
        /// with any number of panels can express distinct sizes (e.g.
        /// `[0.5, 0.3, 0.2]` for three panels) instead of flattening to a
        /// single shared ratio. Adjusted by dragging a split handle; see
        /// [`DockLayout::resize_split_child`].
        ratios: Vec<f32>,
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
            ratios,
            children,
        } = &mut root
        {
            if *direction == axis {
                if before {
                    split_ratio_at(ratios, 0);
                    children.insert(0, DockNode::Panel(panel));
                } else {
                    split_ratio_at(ratios, ratios.len() - 1);
                    children.push(DockNode::Panel(panel));
                }
                self.root = Some(root);
                return true;
            }
        }

        self.root = Some(DockNode::Split {
            direction: axis,
            ratios: vec![0.5, 0.5],
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
        // panel as a sibling rather than nesting a split inside a split. The
        // new panel splits the target child's share in half, so every child
        // keeps a distinct, independently resizable ratio.
        let mut parent = path.0.clone();
        if parent.pop().is_some() {
            let parent_path = NodePath(parent);
            if let Some(DockNode::Split {
                direction,
                ratios,
                children,
                ..
            }) = self.node_at_mut(&parent_path)
            {
                if *direction == axis {
                    let index = *path.0.last().expect("non-root path has a last index");
                    split_ratio_at(ratios, index);
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
            ratios: vec![0.5, 0.5],
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
            DockNode::Split {
                ratios, children, ..
            } => {
                for (index, child) in children.iter_mut().enumerate() {
                    // Only a direct `Panel` leaf (or an emptied node below)
                    // is dropped from this split; the collapsed child's ratio
                    // is dropped with it and the rest renormalized so the
                    // freed space is redistributed proportionally.
                    let is_direct_leaf = matches!(child, DockNode::Panel(_));
                    let removed = match child {
                        DockNode::Panel(id) => *id == panel,
                        other => Self::remove_from_node(other, panel),
                    };
                    if removed {
                        if is_direct_leaf {
                            children.remove(index);
                            if index < ratios.len() {
                                ratios.remove(index);
                            }
                            renormalize_ratios(ratios);
                        }
                        return true;
                    }
                }
                false
            }
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

    /// Sets the share of the first child of the `Split` node at `path` and
    /// redistributes the remaining extent proportionally among the other
    /// children, keeping the `Split.ratios` sum at `1.0`.
    ///
    /// This two-argument form is kept for source compatibility with callers
    /// that only resize a two-panel split (where "first child's share" and
    /// "share of the first pair" coincide). For splits with three or more
    /// children — or when only one boundary should move — use
    /// [`resize_split_child`](Self::resize_split_child), which adjusts a
    /// single pair without touching the others.
    ///
    /// # Panics
    ///
    /// Panics if `path` does not address a [`DockNode::Split`].
    pub fn resize_split(&mut self, path: &NodePath, ratio: f32) {
        let ratio = ratio.clamp(0.05, 0.95);
        let Some(node) = self.node_at_mut(path) else {
            panic!("resize_split: path {path:?} does not address a node");
        };
        let DockNode::Split { ratios, .. } = node else {
            panic!("resize_split: path {path:?} does not address a Split node");
        };
        let rest: f32 = ratios.iter().skip(1).sum();
        let Some(first) = ratios.first_mut() else {
            panic!("resize_split: split at {path:?} has no children");
        };
        *first = ratio;
        if rest > 0.0 {
            let scale = (1.0 - ratio) / rest;
            for share in ratios.iter_mut().skip(1) {
                *share *= scale;
            }
        }
    }

    /// Sets the share of child `index` within its pair (children `index` and
    /// `index + 1`) of the `Split` node at `path`.
    ///
    /// `ratio` is the fraction of the pair's combined extent given to child
    /// `index` (so `0.5` makes the pair even); the two children's entries are
    /// rewritten proportionally and the rest of the split is untouched, so
    /// each panel keeps its own distinct ratio even when the split has three
    /// or more children. `ratio` is clamped to `[0.05, 0.95]` so neither
    /// child can be squeezed out entirely; pixel-level minimum extents (see
    /// the min-size constants in `split_handle`) are enforced by the caller.
    ///
    /// # Panics
    ///
    /// Panics if `path` does not address a [`DockNode::Split`], or if
    /// `index` is not a boundary between two children.
    pub fn resize_split_child(&mut self, path: &NodePath, index: usize, ratio: f32) {
        let Some(node) = self.node_at_mut(path) else {
            panic!("resize_split: path {path:?} does not address a node");
        };
        let DockNode::Split { ratios, .. } = node else {
            panic!("resize_split: path {path:?} does not address a Split node");
        };
        assert!(
            index + 1 < ratios.len(),
            "resize_split_child: boundary {index} out of range for a split with {} children",
            ratios.len(),
        );
        let ratio = ratio.clamp(0.05, 0.95);
        let pair = ratios[index] + ratios[index + 1];
        ratios[index] = ratio * pair;
        ratios[index + 1] = (1.0 - ratio) * pair;
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
                mut ratios,
                children,
            } => {
                let mut clean_children = Vec::new();
                let mut clean_ratios = Vec::new();
                for (index, child) in children.into_iter().enumerate() {
                    // A child that collapsed away (empty tabs, removed leaf)
                    // frees its share; the rest are renormalized below.
                    let Some(child) = Self::cleanup_node(child) else {
                        continue;
                    };
                    let child_share = ratios.get(index).copied().unwrap_or(0.5);
                    match child {
                        DockNode::Split {
                            direction: nested_direction,
                            ratios: nested_ratios,
                            children: nested_children,
                        } if nested_direction == direction => {
                            // Flatten direct same-direction split nests,
                            // scaling the nested ratios by this child's share
                            // so the relative proportions are preserved.
                            for (nested_child, nested_ratio) in
                                nested_children.into_iter().zip(nested_ratios)
                            {
                                clean_children.push(nested_child);
                                clean_ratios.push(child_share * nested_ratio);
                            }
                        }
                        other => {
                            clean_children.push(other);
                            clean_ratios.push(child_share);
                        }
                    }
                }
                // Re-establish the "sum to 1" invariant: drops collapsed
                // children's freed space proportionally and absorbs drift.
                renormalize_ratios(&mut clean_ratios);
                match clean_children.len() {
                    0 => None,
                    1 => Some(clean_children.pop().expect("len == 1")),
                    _ => Some(DockNode::Split {
                        direction,
                        ratios: clean_ratios,
                        children: clean_children,
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

    /// Returns the per-child ratios of the `Split` node at `path`, or `None`
    /// if `path` does not address a split. The entries sum to `1.0`.
    pub fn split_ratios(&self, path: &NodePath) -> Option<Vec<f32>> {
        match self.node_at(path) {
            Some(DockNode::Split { ratios, .. }) => Some(ratios.clone()),
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
        /// See [`DockNode::Split::ratios`].
        ratios: Vec<f32>,
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
    ///
    /// v2: `Split` nodes store a per-child `ratios` vector instead of the
    /// single `ratio` of v1. Snapshots written with v1 cannot be read by a
    /// v2 reader; reject stale versions before restoring.
    pub const VERSION: u32 = 2;

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
                ratios,
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
                        ratios: ratios.clone(),
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
                ratios,
                children,
            } => {
                let children: Vec<DockNode> = children
                    .iter()
                    .filter_map(Self::deserialize_node)
                    .collect();
                match children.len() {
                    0 => None,
                    1 => children.into_iter().next(),
                    _ => {
                        // Evenly sized by default; `cleanup` (run by the
                        // caller) renormalizes and reconciles lengths.
                        let mut ratios = ratios.clone();
                        if ratios.len() != children.len() {
                            ratios = vec![1.0 / children.len() as f32; children.len()];
                        }
                        Some(DockNode::Split {
                            direction: *direction,
                            ratios,
                            children,
                        })
                    }
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

/// Splits the share of child `index` in half and inserts the new child's
/// share right next to it (at position `index`), keeping the sum at `1.0`.
///
/// Used when a panel becomes a sibling of an existing child in a same-axis
/// split: the newcomer takes half of the target child's extent, and the
/// target keeps the other half.
fn split_ratio_at(ratios: &mut Vec<f32>, index: usize) {
    let half = ratios[index] / 2.0;
    ratios[index] = half;
    ratios.insert(index, half);
}

/// Normalizes `ratios` to sum to `1.0`, so freed shares (from removed or
/// collapsed children) are redistributed proportionally and float drift is
/// absorbed. No-op for an empty or all-zero vector.
fn renormalize_ratios(ratios: &mut Vec<f32>) {
    let sum: f32 = ratios.iter().sum();
    if sum > 0.0 {
        for ratio in ratios.iter_mut() {
            *ratio /= sum;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Axis;

    fn panel(id: u64) -> DockNode {
        DockNode::Panel(PanelId::new(id))
    }

    /// Builds `[a] → [a | b] → [a | b | c]` all on one axis, exercising the
    /// sibling-insert path that used to flatten to a single shared ratio.
    fn three_panel_horizontal_layout() -> DockLayout {
        let mut layout = DockLayout::new();
        assert!(layout.insert_panel(PanelId::new(1), None));
        assert!(layout.insert_panel(
            PanelId::new(2),
            Some(DropTarget { panel: Some(PanelId::new(1)), zone: DropZone::Right })
        ));
        assert!(layout.insert_panel(
            PanelId::new(3),
            Some(DropTarget { panel: Some(PanelId::new(2)), zone: DropZone::Right })
        ));
        layout
    }

    #[test]
    fn three_panels_on_one_axis_keep_distinct_ratios() {
        let layout = three_panel_horizontal_layout();
        let root = layout.root().unwrap();
        let DockNode::Split { direction, ratios, children } = root else {
            panic!("expected a single split at the root");
        };
        assert_eq!(*direction, Axis::Horizontal);
        assert_eq!(children.len(), 3);
        // Inserting c to the right of b halves b's share: [1/2, 1/4, 1/4].
        assert_eq!(ratios, &vec![0.5, 0.25, 0.25]);
        let sum: f32 = ratios.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6, "ratios must sum to 1, got {sum}");
    }

    #[test]
    fn resize_split_child_adjusts_only_the_target_pair() {
        let mut layout = three_panel_horizontal_layout();
        let path = NodePath::default();
        // Give the second panel 2/3 of its pair with the third:
        // pair = [0.25, 0.25] → scaled so the second panel holds 2/3.
        layout.resize_split_child(&path, 1, 2.0 / 3.0);
        let ratios = layout.split_ratios(&path).unwrap();
        // The first panel's share is untouched.
        assert!((ratios[0] - 0.5).abs() < 1e-6);
        assert!((ratios[1] - 1.0 / 3.0).abs() < 1e-6);
        assert!((ratios[2] - 1.0 / 6.0).abs() < 1e-6);
        let sum: f32 = ratios.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6);
    }

    #[test]
    fn resize_split_is_source_compatible_and_affects_first_child() {
        let mut layout = three_panel_horizontal_layout();
        // The old two-argument form gives the first child its share of the
        // whole and redistributes the rest proportionally: [1/2, 1/4, 1/4]
        // with ratio 0.7 → [0.7, 0.15, 0.15].
        layout.resize_split(&NodePath::default(), 0.7);
        let ratios = layout.split_ratios(&NodePath::default()).unwrap();
        assert!((ratios[0] - 0.7).abs() < 1e-6);
        assert!((ratios[1] - 0.15).abs() < 1e-6);
        assert!((ratios[2] - 0.15).abs() < 1e-6);
    }

    #[test]
    fn remove_panel_renormalizes_remaining_ratios() {
        let mut layout = three_panel_horizontal_layout();
        // [1/2, 1/4, 1/4] → remove panel 2 → [1/2, 1/4] renormalized to [2/3, 1/3].
        assert!(layout.remove_panel(PanelId::new(2)));
        let root = layout.root().unwrap();
        let DockNode::Split { ratios, children, .. } = root else {
            panic!("expected a split after removal");
        };
        assert_eq!(children.len(), 2);
        assert!((ratios[0] - 2.0 / 3.0).abs() < 1e-6);
        assert!((ratios[1] - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn cleanup_flattens_same_axis_nest_and_scales_ratios() {
        // Hand-construct a same-axis nest: root [a | inner], where inner is
        // itself a horizontal split [b | c] with ratios [3/4, 1/4].
        let mut layout = DockLayout::new();
        layout.root = Some(DockNode::Split {
            direction: Axis::Horizontal,
            ratios: vec![0.5, 0.5],
            children: vec![
                panel(1),
                DockNode::Split {
                    direction: Axis::Horizontal,
                    ratios: vec![0.75, 0.25],
                    children: vec![panel(2), panel(3)],
                },
            ],
        });
        layout.cleanup();
        let root = layout.root().unwrap();
        let DockNode::Split { ratios, children, .. } = root else {
            panic!("expected a flattened split at the root");
        };
        assert_eq!(children.len(), 3);
        // Inner ratios scaled by the inner node's share: [0.5, 0.375, 0.125].
        assert!((ratios[0] - 0.5).abs() < 1e-6);
        assert!((ratios[1] - 0.375).abs() < 1e-6);
        assert!((ratios[2] - 0.125).abs() < 1e-6);
    }

    #[test]
    fn layout_state_round_trips_distinct_ratios() {
        let mut layout = three_panel_horizontal_layout();
        layout.resize_split_child(&NodePath::default(), 1, 0.8);
        let registry = TestRegistry;
        let state = DockLayoutState::capture(&layout, &registry);
        assert_eq!(state.version, DockLayoutState::VERSION);
        let restored = state.to_layout();
        // Panel ids are re-derived from registry keys on restore, so compare
        // the tree *shape* (direction, ratios, structure) rather than ids.
        let DockNode::Split {
            direction,
            ratios,
            children,
        } = restored.root().unwrap()
        else {
            panic!("expected a split at the restored root");
        };
        assert_eq!(*direction, Axis::Horizontal);
        let original_ratios = layout.split_ratios(&NodePath::default()).unwrap();
        assert_eq!(ratios, &original_ratios);
        assert_eq!(children.len(), 3);
    }

    #[test]
    fn insert_before_splits_the_target_child() {
        let mut layout = three_panel_horizontal_layout();
        // [1/2, 1/4, 1/4]; inserting d to the LEFT of panel 2 halves panel 2's
        // share and puts d in front of it.
        assert!(layout.insert_panel(
            PanelId::new(4),
            Some(DropTarget { panel: Some(PanelId::new(2)), zone: DropZone::Left })
        ));
        let ratios = layout.split_ratios(&NodePath::default()).unwrap();
        assert_eq!(ratios, vec![0.5, 0.125, 0.125, 0.25]);
    }

    struct TestRegistry;
    impl PanelRegistry for TestRegistry {
        fn panel_key(&self, id: PanelId) -> Option<String> {
            Some(format!("panel-{}", id.raw()))
        }
        fn build_panel(&self, _key: &str, _window: &mut Window, _cx: &mut App) -> Option<PanelHandle> {
            None
        }
    }
}
