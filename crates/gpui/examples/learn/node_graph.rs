//! Intended-usage sketch for the `gpui::node_graph` node-graph editor.
//!
//! Builds a mock video pipeline (media → transform → output), implements the
//! data-source traits over it, and subscribes to the view's edit-request
//! events, logging each one. The real Oak integration maps these events onto
//! engine operations wrapped in undo commands — see the "Wiring into Oak"
//! section of [`gpui::node_graph`].
//!
//! NOTE: the widget itself is still an API skeleton (`todo!()` bodies), so
//! running this example will panic as soon as the view renders. It exists to
//! pin down the intended usage and keep it compiling.

// The modules under demo are skeletons whose bodies are `todo!()` by design.
#![allow(clippy::todo)]

use gpui::node_graph::{
	EdgeData, EdgeId, NodeData, NodeGraphDataSource, NodeGraphEvent, NodeGraphView, NodeId,
	PortData, PortDataType, PortId, PortKind,
};
use gpui::{
	App, Bounds, Context, Entity, Hsla, Pixels, Point, Render, SharedString, Window, WindowBounds,
	WindowOptions, div, point, prelude::*, px, size,
};

// ---------------------------------------------------------------------------
// Mock graph data
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct MockPort {
	id: PortId,
	kind: PortKind,
	label: &'static str,
	data_type: PortDataType,
	connected: bool,
}

impl PortData for MockPort {
	fn id(&self) -> PortId {
		self.id
	}
	fn kind(&self) -> PortKind {
		self.kind
	}
	fn label(&self) -> SharedString {
		self.label.into()
	}
	fn data_type(&self) -> PortDataType {
		self.data_type.clone()
	}
	fn is_connected(&self) -> bool {
		self.connected
	}
}

#[derive(Clone)]
struct MockNode {
	id: NodeId,
	title: &'static str,
	position: Point<Pixels>,
	inputs: Vec<MockPort>,
	outputs: Vec<MockPort>,
	header_color: Option<Hsla>,
}

impl NodeData for MockNode {
	type Port = MockPort;

	fn id(&self) -> NodeId {
		self.id
	}
	fn title(&self) -> SharedString {
		self.title.into()
	}
	fn position(&self) -> Point<Pixels> {
		self.position
	}
	fn inputs(&self) -> Vec<MockPort> {
		self.inputs.clone()
	}
	fn outputs(&self) -> Vec<MockPort> {
		self.outputs.clone()
	}
	fn header_color(&self) -> Option<Hsla> {
		self.header_color
	}
	fn is_collapsed(&self) -> bool {
		false
	}
	fn is_enabled(&self) -> bool {
		true
	}
}

#[derive(Clone)]
struct MockEdge {
	id: EdgeId,
	from_node: NodeId,
	from_port: PortId,
	to_node: NodeId,
	to_port: PortId,
}

impl EdgeData for MockEdge {
	fn id(&self) -> EdgeId {
		self.id
	}
	fn from_node(&self) -> NodeId {
		self.from_node
	}
	fn from_port(&self) -> PortId {
		self.from_port
	}
	fn to_node(&self) -> NodeId {
		self.to_node
	}
	fn to_port(&self) -> PortId {
		self.to_port
	}
}

/// The mock pipeline: media → transform → output.
struct MockGraph {
	nodes: Vec<MockNode>,
	edges: Vec<MockEdge>,
}

impl MockGraph {
	fn new() -> Self {
		let video = PortDataType::new("video", Hsla::blue());
		// Port id packing: node id in the high bits, port index low. Inputs
		// and outputs share one index space per node.
		let port = |node: u64, index: u64| PortId((node << 32) | index);

		let media = MockNode {
			id: NodeId(1),
			title: "Media",
			position: point(px(40.), px(80.)),
			inputs: vec![],
			outputs: vec![MockPort {
				id: port(1, 0),
				kind: PortKind::Output,
				label: "video",
				data_type: video.clone(),
				connected: true,
			}],
			header_color: Some(Hsla::green()),
		};
		let transform = MockNode {
			id: NodeId(2),
			title: "Transform",
			position: point(px(320.), px(140.)),
			inputs: vec![MockPort {
				id: port(2, 0),
				kind: PortKind::Input,
				label: "in",
				data_type: video.clone(),
				connected: true,
			}],
			outputs: vec![MockPort {
				id: port(2, 1),
				kind: PortKind::Output,
				label: "out",
				data_type: video.clone(),
				connected: true,
			}],
			header_color: None,
		};
		let output = MockNode {
			id: NodeId(3),
			title: "Output",
			position: point(px(600.), px(200.)),
			inputs: vec![MockPort {
				id: port(3, 0),
				kind: PortKind::Input,
				label: "in",
				data_type: video,
				connected: true,
			}],
			outputs: vec![],
			header_color: Some(Hsla::red()),
		};

		let edges = vec![
			MockEdge {
				id: EdgeId(1),
				from_node: NodeId(1),
				from_port: port(1, 0),
				to_node: NodeId(2),
				to_port: port(2, 0),
			},
			MockEdge {
				id: EdgeId(2),
				from_node: NodeId(2),
				from_port: port(2, 1),
				to_node: NodeId(3),
				to_port: port(3, 0),
			},
		];

		Self {
			nodes: vec![media, transform, output],
			edges,
		}
	}
}

impl NodeGraphDataSource for MockGraph {
	type Node = MockNode;
	type Edge = MockEdge;

	fn nodes(&self) -> Vec<MockNode> {
		self.nodes.clone()
	}
	fn edges(&self) -> Vec<MockEdge> {
		self.edges.clone()
	}
	fn can_connect(&self, _from: PortId, _to: PortId) -> bool {
		// A real app checks type compatibility, cycles and cardinality here.
		// The mock allows everything between distinct ports.
		true
	}
}

// ---------------------------------------------------------------------------
// App view: hosts the graph view and logs edit requests
// ---------------------------------------------------------------------------

struct NodeGraphExample {
	graph: Entity<MockGraph>,
	view: Entity<NodeGraphView<MockGraph>>,
}

impl NodeGraphExample {
	fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
		let graph = cx.new(|_cx| MockGraph::new());
		let view = cx.new(|cx| NodeGraphView::new(graph.clone(), window, cx));

		// In Oak, each event becomes an engine operation wrapped in an undo
		// command; here we just log the request.
		cx.subscribe(&view, |_this, _view, event: &NodeGraphEvent, cx| {
			println!("[node_graph] edit request: {event:?}");
			// After applying a request to the model, notify so the view
			// re-reads it, e.g.: `graph.update(cx, |_, cx| cx.notify())`.
			let _ = cx;
		})
		.detach();

		Self { graph, view }
	}
}

impl Render for NodeGraphExample {
	fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
		let _ = &self.graph;
		div().size_full().child(self.view.clone())
	}
}

fn main() {
	gpui_platform::application().run(|cx: &mut App| {
		let bounds = Bounds::centered(None, size(px(900.), px(600.)), cx);
		cx.open_window(
			WindowOptions {
				window_bounds: Some(WindowBounds::Windowed(bounds)),
				..Default::default()
			},
			|window, cx| cx.new(|cx| NodeGraphExample::new(window, cx)),
		)
		.expect("failed to open window");
		cx.activate(true);
	});
}
