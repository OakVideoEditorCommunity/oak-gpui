//! Intended-usage sketch of the `gpui::effect_stack` widget.
//!
//! Builds a mock stack (Media → Transform → OCIO LUT → Output), hosts an
//! [`EffectStackView`], and logs every edit request it emits. The mock data
//! source simply applies requests in place and calls `cx.notify()`; a real
//! app (Oak) would route them through its engine and undo stack.
//!
//! NOTE: the widget implementation itself is still a skeleton (`todo!()`),
//! so this example compiles but does not render yet.

// The modules under demo are skeletons whose bodies are `todo!()` by design.
#![allow(clippy::todo)]

#[path = "../shared/prelude.rs"]
mod example_prelude;

use std::sync::Arc;

use example_prelude::init_example;
use gpui::effect_stack::{
	EffectCardKind, EffectData, EffectId, EffectStackDataSource, EffectStackEvent, EffectStackView,
};
use gpui::{
	App, Bounds, Context, Entity, Render, SharedString, Window, WindowBounds, WindowOptions, div,
	prelude::*, px, size,
};

// ---------------------------------------------------------------------------
// Mock model
// ---------------------------------------------------------------------------

struct MockEffect {
	id: EffectId,
	kind: EffectCardKind,
	title: String,
	subtitle: Option<String>,
	enabled: bool,
	expanded: bool,
	badge: Option<usize>,
}

impl EffectData for MockEffect {
	fn id(&self) -> EffectId {
		self.id
	}
	fn kind(&self) -> EffectCardKind {
		self.kind
	}
	fn title(&self) -> SharedString {
		self.title.clone().into()
	}
	fn subtitle(&self) -> Option<SharedString> {
		self.subtitle.clone().map(Into::into)
	}
	fn is_enabled(&self) -> bool {
		self.enabled
	}
	fn is_expanded(&self) -> bool {
		self.expanded
	}
	fn badge_count(&self) -> Option<usize> {
		self.badge
	}
}

/// The mock data source. In Oak this would be a view-model entity deriving
/// the ordered card list from the node-graph path of the selected clip.
struct MockStack {
	clip_name: String,
	effects: Vec<MockEffect>,
	/// Reserved for allocating ids to effects added at runtime.
	#[allow(dead_code)]
	next_id: u64,
}

impl MockStack {
	fn demo() -> Self {
		Self {
			clip_name: "A001_C002_0103.mov".to_string(),
			effects: vec![
				MockEffect {
					id: EffectId(0),
					kind: EffectCardKind::Source,
					title: "Media".into(),
					subtitle: Some("A001_C002_0103.mov".into()),
					enabled: true,
					expanded: false,
					badge: None,
				},
				MockEffect {
					id: EffectId(1),
					kind: EffectCardKind::Effect,
					title: "Transform".into(),
					subtitle: Some("scale 100%, rotate 0°".into()),
					enabled: true,
					expanded: true,
					badge: Some(2),
				},
				MockEffect {
					id: EffectId(2),
					kind: EffectCardKind::Effect,
					title: "OCIO LUT".into(),
					subtitle: Some("filmic_to_display.cube".into()),
					enabled: true,
					expanded: false,
					badge: None,
				},
				MockEffect {
					id: EffectId(3),
					kind: EffectCardKind::Output,
					title: "Output".into(),
					subtitle: None,
					enabled: true,
					expanded: false,
					badge: None,
				},
			],
			next_id: 4,
		}
	}
}

impl EffectStackDataSource for MockStack {
	fn effects(&self) -> Vec<Arc<dyn EffectData>> {
		self.effects
			.iter()
			.map(|effect| {
				Arc::new(MockEffect {
					id: effect.id,
					kind: effect.kind,
					title: effect.title.clone(),
					subtitle: effect.subtitle.clone(),
					enabled: effect.enabled,
					expanded: effect.expanded,
					badge: effect.badge,
				}) as Arc<dyn EffectData>
			})
			.collect()
	}

	fn target_label(&self) -> Option<SharedString> {
		Some(self.clip_name.clone().into())
	}
}

// ---------------------------------------------------------------------------
// Root view: hosts the stack and applies edit requests to the mock model.
// ---------------------------------------------------------------------------

/// Placeholder parameter view. A real app builds the effect's controls here
/// and calls [`EffectStackView::notify_parameter_changed`] after edits.
struct MockParams {
	effect: EffectId,
}

impl Render for MockParams {
	fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
		div().child(format!("parameters for {} (mock)", self.effect))
	}
}

struct StackDemoRoot {
	data: Entity<MockStack>,
	stack: Entity<EffectStackView<MockStack>>,
}

impl StackDemoRoot {
	fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
		let data = cx.new(|_cx| MockStack::demo());

		let stack = cx.new(|cx| {
			EffectStackView::new(data.clone(), cx).params_renderer(|id, _window, cx| {
				// Real apps build the effect's parameter controls here. The
				// mock just shows a placeholder label.
				cx.new(|_cx| MockParams { effect: *id }).into()
			})
		});

		// The "edits are requests" loop: log each request, apply it to the
		// model (Oak: engine command + undo), then notify.
		cx.subscribe_in(&stack, window, {
			let data = data.clone();
			move |_root, _stack, event: &EffectStackEvent, _window, cx| {
				println!("[effect_stack] request: {event:?}");
				data.update(cx, |data, cx| {
					match event {
						EffectStackEvent::EnableToggled { effect, enabled } => {
							if let Some(e) = data.effects.iter_mut().find(|e| e.id == *effect) {
								e.enabled = *enabled;
							}
						}
						EffectStackEvent::ExpansionToggled { effect, expanded } => {
							if let Some(e) = data.effects.iter_mut().find(|e| e.id == *effect) {
								e.expanded = *expanded;
							}
						}
						EffectStackEvent::ReorderRequested { .. }
						| EffectStackEvent::RemoveRequested(_)
						| EffectStackEvent::AddRequested { .. }
						| EffectStackEvent::ContextMenuRequested { .. }
						| EffectStackEvent::ParameterChanged { .. } => {
							todo!("apply {event:?} to the mock model (or engine, in a real app)")
						}
					}
					cx.notify();
				});
			}
		})
		.detach();

		Self { data, stack }
	}
}

impl Render for StackDemoRoot {
	fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
		let _ = &self.data;
		let _ = cx;
		div()
			.size_full()
			.flex()
			.items_center()
			.justify_center()
			.child(self.stack.clone())
	}
}

fn main() {
	gpui_platform::application().run(|cx: &mut App| {
		init_example(cx, "Effect Stack");

		let bounds = Bounds::centered(None, size(px(420.0), px(640.0)), cx);
		cx.open_window(
			WindowOptions {
				window_bounds: Some(WindowBounds::Windowed(bounds)),
				..Default::default()
			},
			|window, cx| cx.new(|cx| StackDemoRoot::new(window, cx)),
		)
		.unwrap();
	});
}
