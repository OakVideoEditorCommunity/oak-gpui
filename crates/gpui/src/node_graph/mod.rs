//! Node-graph editor widget.
//!
//! This module provides a data-agnostic, interactive node-graph editor in the
//! style of compositing / video-editing tools (Nuke, Blender, DaVinci Fusion).
//! It is built for the Oak video editor but knows nothing about Oak's engine:
//! all graph data is supplied by the embedding application through traits, and
//! every user edit is surfaced as a *request* event rather than being applied
//! directly.
//!
//! # Architecture
//!
//! - **Trait-driven data source.** The widget never owns graph data. The app
//!   implements [`NodeGraphDataSource`], [`NodeData`], [`PortData`] and
//!   [`EdgeData`] (see [`data`](crate::node_graph::data)) over its own model and hands the view an
//!   `Entity<D>`. After the app mutates its model it calls `cx.notify()` on
//!   the data entity and the view re-reads everything on the next frame.
//! - **Canvas with pan/zoom.** [`GraphViewState`] (see [`state`](crate::node_graph::state)) holds the
//!   viewport (`offset`, `zoom`) and the current selection, plus the pure
//!   coordinate transforms between *graph space* (the document coordinate
//!   system node positions live in) and *screen space*.
//! - **Edits as requests.** Moving nodes, connecting ports, deleting items —
//!   none of these mutate the graph directly. The view emits
//!   [`NodeGraphEvent`]s (see [`graph_view`](crate::node_graph::graph_view)); the app validates them against
//!   its engine and its undo stack, applies them, and notifies. This keeps the
//!   app's engine the single source of truth and makes undo/redo trivial.
//! - **App-supplied connection rules.** Type compatibility, cycle prevention
//!   and port cardinality are enforced by the app via
//!   [`NodeGraphDataSource::can_connect`]. The widget calls it live during
//!   wire drags to highlight valid drop targets, and again on drop before
//!   emitting [`NodeGraphEvent::ConnectionRequested`].
//!
//! # Submodules
//!
//! - [`data`](crate::node_graph::data) — identifier newtypes and the data-source traits.
//! - [`state`](crate::node_graph::state) — viewport/selection state and coordinate math.
//! - [`graph_view`](crate::node_graph::graph_view) — the [`NodeGraphView`] view and [`NodeGraphEvent`].
//! - [`node_element`](crate::node_graph::node_element) — rendering of a single node card.
//! - [`wire`](crate::node_graph::wire) — bezier wire rendering, including the drag "ghost" wire.
//! - [`minimap`](crate::node_graph::minimap) — overview minimap (backdrop + viewport indicator).
//!
//! # Wiring into Oak
//!
//! Oak's engine (`oakengine`) owns the real node graph (media → transform →
//! OCIO LUT → output, …). The intended integration:
//!
//! | Widget event | Engine operation |
//! |---|---|
//! | [`NodeGraphEvent::NodeMovePreview`] / [`NodeGraphEvent::NodeMoveRequested`] | transient UI feedback / `engine.move_nodes(...)` wrapped in an undo command |
//! | [`NodeGraphEvent::ConnectionRequested`] | `engine.connect(from, to)` (engine re-validates type & cycle rules) |
//! | [`NodeGraphEvent::DisconnectionRequested`] | `engine.disconnect(edge)` |
//! | [`NodeGraphEvent::DeleteRequested`] | `engine.remove(nodes, edges)` as one undo step |
//! | [`NodeGraphEvent::BackgroundClicked`] | open the "add node" menu at the given graph position |
//!
//! The companion [`crate::effect_stack`] module shows the *same* engine graph
//! as a linear effect stack. The two views are exactly that — two views over
//! one model: they share the engine's node identities ([`NodeId`] is typically
//! a newtype over the engine's node key), so selection sync between them is a
//! matter of storing one shared selection set in the app, not of data
//! conversion. Edits made in either view go through the same engine ops and
//! undo stack.
//!
//! [`NodeGraphDataSource`]: crate::node_graph::NodeGraphDataSource
//! [`NodeGraphDataSource::can_connect`]: crate::node_graph::NodeGraphDataSource::can_connect
//! [`NodeData`]: crate::node_graph::NodeData
//! [`PortData`]: crate::node_graph::PortData
//! [`EdgeData`]: crate::node_graph::EdgeData
//! [`NodeId`]: crate::node_graph::NodeId
//! [`GraphViewState`]: crate::node_graph::GraphViewState
//! [`NodeGraphView`]: crate::node_graph::NodeGraphView
//! [`NodeGraphEvent`]: crate::node_graph::NodeGraphEvent
//! [`NodeGraphEvent::NodeMovePreview`]: crate::node_graph::NodeGraphEvent::NodeMovePreview
//! [`NodeGraphEvent::NodeMoveRequested`]: crate::node_graph::NodeGraphEvent::NodeMoveRequested
//! [`NodeGraphEvent::ConnectionRequested`]: crate::node_graph::NodeGraphEvent::ConnectionRequested
//! [`NodeGraphEvent::DisconnectionRequested`]: crate::node_graph::NodeGraphEvent::DisconnectionRequested
//! [`NodeGraphEvent::DeleteRequested`]: crate::node_graph::NodeGraphEvent::DeleteRequested
//! [`NodeGraphEvent::BackgroundClicked`]: crate::node_graph::NodeGraphEvent::BackgroundClicked

pub mod data;
pub mod graph_view;
pub mod minimap;
pub mod node_element;
pub mod state;
pub mod wire;

pub use data::*;
pub use graph_view::*;
pub use minimap::*;
pub use node_element::*;
pub use state::*;
pub use wire::*;
