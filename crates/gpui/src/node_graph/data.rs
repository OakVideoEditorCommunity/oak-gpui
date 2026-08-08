//! Data-source traits and identifier types for the node-graph editor.
//!
//! The widget is fully data-agnostic: it reads everything it displays through
//! the traits in this file and never mutates the underlying model. The
//! embedding application implements these traits over its own graph (for Oak:
//! the `oakengine` node graph) and reacts to the [`NodeGraphEvent`]s emitted
//! by the view.
//!
//! [`NodeGraphEvent`]: crate::node_graph::NodeGraphEvent

use crate::{Hsla, Pixels, Point, SharedString};

/// Unique identifier of a node within the graph.
///
/// Typically a newtype over the host application's own node key (e.g. an
/// engine node handle). The widget only requires that ids are cheap to copy,
/// totally ordered (for selection sets) and hashable (for lookup maps).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u64);

/// Unique identifier of a port within the graph.
///
/// Port ids are *globally* unique, not per-node, so that a single [`PortId`]
/// is enough to address an endpoint of a connection request. The app is free
/// to pack a node id and a per-node port index into the `u64` however it
/// likes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PortId(pub u64);

/// Unique identifier of an edge (a connection between two ports).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EdgeId(pub u64);

/// Whether a port accepts incoming connections or produces outgoing ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PortKind {
    /// A port that consumes data; conventionally drawn on the left side of a
    /// node and accepts connections *from* an [`PortKind::Output`] port.
    Input,
    /// A port that produces data; conventionally drawn on the right side of a
    /// node and connects *to* an [`PortKind::Input`] port.
    Output,
}

/// A lightweight, app-defined descriptor of the data flowing through a port.
///
/// The widget does not interpret data types semantically — it uses the
/// [`color`](Self::color) to tint port dots and wires, and uses
/// [`PartialEq`] only as a convenience for *default* visual hints. The
/// authoritative compatibility check is always
/// [`NodeGraphDataSource::can_connect`], so an app may implement subtyping,
/// implicit conversions (e.g. `int → float`) or direction-dependent rules
/// there without this type needing to model them.
///
/// # Equality contract
///
/// Two [`PortDataType`] values are considered the same type when their
/// `name`s are equal; the color is *not* part of equality. Apps that want
/// distinct types sharing a name should disambiguate the name.
#[derive(Clone, Debug)]
pub struct PortDataType {
    /// Human-readable type name, e.g. `"video"`, `"audio"`, `"matte"`.
    /// Also used as the identity of the type (see type-level docs).
    pub name: SharedString,
    /// Color used to tint port dots and wires carrying this type.
    pub color: Hsla,
}

impl PortDataType {
    /// Creates a new data-type descriptor with the given display name and
    /// tint color.
    pub fn new(name: impl Into<SharedString>, color: Hsla) -> Self {
        Self {
            name: name.into(),
            color,
        }
    }
}

impl PartialEq for PortDataType {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for PortDataType {}

/// A single port on a node.
///
/// Ports are the endpoints of edges. Each port has a globally unique
/// [`PortId`], a direction ([`PortKind`]) and a [`PortDataType`] used for
/// tinting.
pub trait PortData {
    /// Returns the globally unique identifier of this port.
    fn id(&self) -> PortId;

    /// Returns whether this is an input or an output port.
    fn kind(&self) -> PortKind;

    /// Returns the short label drawn next to the port dot (e.g. `"in"`,
    /// `"mask"`). May be empty, in which case only the dot is drawn.
    fn label(&self) -> SharedString;

    /// Returns the data type of this port, used to tint the port dot and any
    /// wires connected to it.
    fn data_type(&self) -> PortDataType;

    /// Returns whether this port currently has at least one edge attached.
    ///
    /// Used only for rendering (connected dots are filled, unconnected dots
    /// are hollow) and for styling during wire drags; the widget does not
    /// enforce any cardinality rules from it — that is the job of
    /// [`NodeGraphDataSource::can_connect`].
    fn is_connected(&self) -> bool;
}

/// A single node in the graph.
///
/// Nodes are rectangular cards with a header, a column of input ports on the
/// left and a column of output ports on the right (see
/// [`NodeElement`](crate::node_graph::NodeElement)).
pub trait NodeData {
    /// The port type used by this node's inputs and outputs.
    type Port: PortData;

    /// Returns the unique identifier of this node.
    fn id(&self) -> NodeId;

    /// Returns the title drawn in the node's header.
    fn title(&self) -> SharedString;

    /// Returns the position of the node's top-left corner in *graph space*
    /// (the document coordinate system).
    ///
    /// Graph space is an unbounded, zoom-independent coordinate system: a
    /// node at `point(px(100.), px(40.))` stays attached to that document
    /// location regardless of pan and zoom. The view converts to screen
    /// coordinates with
    /// [`GraphViewState::graph_to_screen`](crate::node_graph::GraphViewState::graph_to_screen).
    fn position(&self) -> Point<Pixels>;

    /// Returns the input ports of this node, in top-to-bottom draw order.
    fn inputs(&self) -> Vec<Self::Port>;

    /// Returns the output ports of this node, in top-to-bottom draw order.
    fn outputs(&self) -> Vec<Self::Port>;

    /// Returns an optional accent color for the node header, or `None` to use
    /// the theme default. Apps typically use this to group nodes by category
    /// (inputs, transforms, color management, outputs, …).
    fn header_color(&self) -> Option<Hsla>;

    /// Returns whether the node is collapsed to just its header.
    ///
    /// Collapsed nodes draw no ports and cannot be connection targets. The
    /// collapsed state itself belongs to the app's model (or view state); the
    /// widget only reflects it.
    fn is_collapsed(&self) -> bool;

    /// Returns whether the node is enabled.
    ///
    /// Disabled nodes (e.g. a bypassed effect) are drawn dimmed. This is a
    /// purely visual hint; the widget does not change interaction behavior
    /// for disabled nodes.
    fn is_enabled(&self) -> bool;
}

/// A single directed connection from an output port to an input port.
pub trait EdgeData {
    /// Returns the unique identifier of this edge.
    fn id(&self) -> EdgeId;

    /// Returns the id of the node the connection starts at.
    fn from_node(&self) -> NodeId;

    /// Returns the id of the output port the connection starts at.
    fn from_port(&self) -> PortId;

    /// Returns the id of the node the connection ends at.
    fn to_node(&self) -> NodeId;

    /// Returns the id of the input port the connection ends at.
    fn to_port(&self) -> PortId;
}

/// The data source backing a [`NodeGraphView`](crate::node_graph::NodeGraphView).
///
/// The app implements this trait over its engine model and places it in an
/// `Entity`. The view re-reads `nodes()` and `edges()` every frame in which
/// the entity notifies, so implementations should be cheap snapshots or
/// borrow from cached data.
///
/// All methods take `&self`; the widget never mutates the source. Edits
/// arrive back at the app as [`NodeGraphEvent`]s.
///
/// [`NodeGraphEvent`]: crate::node_graph::NodeGraphEvent
pub trait NodeGraphDataSource {
    /// The node type returned by [`nodes()`](Self::nodes).
    type Node: NodeData;
    /// The edge type returned by [`edges()`](Self::edges).
    type Edge: EdgeData;

    /// Returns all nodes to display, in no required order (the view sorts for
    /// painting; selection order is unaffected).
    fn nodes(&self) -> Vec<Self::Node>;

    /// Returns all edges to display. Edges referencing ports or nodes that
    /// are not part of [`nodes()`](Self::nodes) are ignored by the view.
    fn edges(&self) -> Vec<Self::Edge>;

    /// Returns whether connecting output port `from` to input port `to`
    /// would be valid.
    ///
    /// This is the single place where the app enforces its connection rules:
    /// data-type compatibility (including implicit conversions), cycle
    /// prevention, port cardinality, node enablement, and so on. The view
    /// calls this:
    ///
    /// - *continuously during a wire drag* to highlight compatible target
    ///   ports and to mark the ghost wire as valid/invalid, and
    /// - *once on drop* before emitting
    ///   [`NodeGraphEvent::ConnectionRequested`](crate::node_graph::NodeGraphEvent::ConnectionRequested)
    ///   — a drop on a port for which this returns `false` cancels the drag
    ///   silently.
    ///
    /// It must be cheap, pure, and consistent: the same arguments must yield
    /// the same answer within a frame. The view passes output port first,
    /// input port second, regardless of which end the user started the drag
    /// from. Returning `true` here does not commit the app to accepting the
    /// connection; the engine may still reject it when the event arrives
    /// (e.g. it raced with another edit), in which case the app simply does
    /// not apply it.
    fn can_connect(&self, from: PortId, to: PortId) -> bool;
}
