//! Dock Layout Example (intended-usage sketch)
//!
//! Sketch of how the Oak video editor will wire up `gpui::dock`: an app with
//! a single [`DockArea`] hosting four placeholder panels — project bin,
//! viewer, inspector, and timeline — plus a [`PanelRegistry`] so layouts can
//! be saved and restored.
//!
//! NOTE: the dock implementation is not finished yet (all of its methods are
//! `todo!()`), so this example compiles but panics at runtime until the
//! implementation lands.

// The modules under demo are skeletons whose bodies are `todo!()` by design.
#![allow(clippy::todo)]

#[path = "../shared/prelude.rs"]
mod example_prelude;

use example_prelude::init_example;
use gpui::dock::{
	DockArea, DockEvent, DockLayoutState, DockPanel, PanelEvent, PanelHandle, PanelId,
	PanelRegistry,
};
use gpui::{
	AnyElement, App, Bounds, Context, Entity, EventEmitter, Render, SharedString, Window,
	WindowBounds, WindowOptions, div, prelude::*, px, size,
};
use std::sync::Arc;

// ============================================================================
// Demo panels
//
// Each placeholder is a normal GPUI view plus a `DockPanel` impl. The panel
// ids double as the persistence mapping below.
// ============================================================================

const PROJECT_BIN_ID: PanelId = PanelId::new(1);
const VIEWER_ID: PanelId = PanelId::new(2);
const INSPECTOR_ID: PanelId = PanelId::new(3);
const TIMELINE_ID: PanelId = PanelId::new(4);

/// Shared shape of the demo placeholders: a labeled box.
struct PlaceholderPanel {
	id: PanelId,
	title: &'static str,
}

impl PlaceholderPanel {
	fn new(id: PanelId, title: &'static str) -> Self {
		Self { id, title }
	}
}

impl Render for PlaceholderPanel {
	fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
		div()
			.size_full()
			.flex()
			.items_center()
			.justify_center()
			.child(format!("{} (placeholder)", self.title))
	}
}

impl EventEmitter<PanelEvent> for PlaceholderPanel {}

impl DockPanel for PlaceholderPanel {
	fn panel_id(&self) -> PanelId {
		self.id
	}

	fn title(&self, _cx: &App) -> SharedString {
		self.title.into()
	}

	fn tab_content(&self, _cx: &App) -> AnyElement {
		div().child(self.title).into_any_element()
	}
}

// ============================================================================
// Panel registry: bridges string keys (persisted) and live panel views.
// ============================================================================

struct DemoPanelRegistry;

impl PanelRegistry for DemoPanelRegistry {
	fn panel_key(&self, id: PanelId) -> Option<String> {
		match id {
			PROJECT_BIN_ID => Some("project-bin".into()),
			VIEWER_ID => Some("viewer".into()),
			INSPECTOR_ID => Some("inspector".into()),
			TIMELINE_ID => Some("timeline".into()),
			_ => None,
		}
	}

	fn build_panel(&self, key: &str, _window: &mut Window, cx: &mut App) -> Option<PanelHandle> {
		let (id, title) = match key {
			"project-bin" => (PROJECT_BIN_ID, "Project Bin"),
			"viewer" => (VIEWER_ID, "Viewer"),
			"inspector" => (INSPECTOR_ID, "Inspector"),
			"timeline" => (TIMELINE_ID, "Timeline"),
			_ => return None,
		};
		Some(PanelHandle::new(
			cx.new(|_| PlaceholderPanel::new(id, title)),
			cx,
		))
	}
}

// ============================================================================
// Root view: just hosts the dock area and logs its events.
// ============================================================================

struct DockLayoutExample {
	dock: Entity<DockArea>,
}

impl DockLayoutExample {
	fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
		let dock = cx.new(|cx| DockArea::new(cx).with_registry(Arc::new(DemoPanelRegistry)));

		// Seed a default workspace. Once the dock is implemented this will
		// instead attempt `restore_state` from a persisted `DockLayoutState`
		// first, falling back to this default when none exists.
		let panels: Vec<PanelHandle> = ["project-bin", "viewer", "inspector", "timeline"]
			.into_iter()
			.filter_map(|key| DemoPanelRegistry.build_panel(key, window, cx))
			.collect();
		dock.update(cx, |dock, cx| {
			for panel in panels {
				dock.add_panel(panel, None, cx);
			}
		});

		// Autosave hook: persist on every layout change.
		cx.subscribe(
			&dock,
			|_this, dock: Entity<DockArea>, event: &DockEvent, cx| {
				if let DockEvent::LayoutChanged = event {
					let _state: DockLayoutState = dock.read(cx).save_state();
					todo!("serialize `_state` with serde_json and write it to the app data dir");
				}
			},
		)
		.detach();

		Self { dock }
	}
}

impl Render for DockLayoutExample {
	fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
		div().size_full().child(self.dock.clone())
	}
}

fn main() {
	gpui_platform::application().run(|cx: &mut App| {
		let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
		cx.open_window(
			WindowOptions {
				window_bounds: Some(WindowBounds::Windowed(bounds)),
				..Default::default()
			},
			|window, cx| cx.new(|cx| DockLayoutExample::new(window, cx)),
		)
		.expect("Failed to open window");

		init_example(cx, "Dock Layout");
	});
}
