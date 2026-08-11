//! The material bin (project explorer): a tree or icon view of the project's
//! media, with file-drop support.
//!
//! Data-agnostic: the host implements [`ProjectDataSource`] over its model;
//! the widget only *requests* — opening an entry or importing dropped files
//! is emitted as a [`ProjectExplorerEvent`] for the host to apply. Thumbnails
//! are asset paths rendered with `img`; without one the entry shows a
//! placeholder swatch.

use gpui::{
	App, ClickEvent, Context, ElementId, Entity, EventEmitter, ExternalPaths, FocusHandle,
	Focusable, Hsla, Render, SharedString, Window, colors::DefaultColors, div, img, prelude::*, px,
};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

/// A fully transparent color (for un-hovered rows).
fn transparent() -> gpui::Rgba {
	gpui::Rgba {
		r: 0.0,
		g: 0.0,
		b: 0.0,
		a: 0.0,
	}
}

/// A single entry in the project tree.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectEntry {
	/// The entry's stable id (host-side key).
	pub id: u64,
	/// The display name.
	pub name: SharedString,
	/// Whether the entry is a folder (expandable).
	pub is_dir: bool,
	/// An asset path for a thumbnail, if any.
	pub thumbnail: Option<SharedString>,
}

impl ProjectEntry {
	/// Create an entry.
	pub fn new(id: u64, name: impl Into<SharedString>, is_dir: bool) -> Self {
		Self {
			id,
			name: name.into(),
			is_dir,
			thumbnail: None,
		}
	}

	/// Attach a thumbnail asset path.
	pub fn with_thumbnail(mut self, thumbnail: impl Into<SharedString>) -> Self {
		self.thumbnail = Some(thumbnail.into());
		self
	}
}

/// The host's project model, read through this trait.
pub trait ProjectDataSource: 'static {
	/// The top-level entries.
	fn roots(&self) -> Vec<ProjectEntry>;
	/// The children of a folder entry.
	fn children(&self, parent_id: u64) -> Vec<ProjectEntry>;
}

/// The material bin's display mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplorerView {
	/// A hierarchical tree.
	Tree,
	/// A flat grid of thumbnails.
	Icons,
}

/// A request emitted by the explorer.
#[derive(Debug, Clone, PartialEq)]
pub enum ProjectExplorerEvent {
	/// Open an entry (double-click on a file, or an explicit open request).
	OpenRequested {
		/// The explorer's stable id.
		control: usize,
		/// The entry's id.
		id: u64,
		/// The entry's name.
		name: SharedString,
	},
	/// Files were dropped onto the bin.
	FileDropRequested {
		/// The explorer's stable id.
		control: usize,
		/// The dropped paths.
		paths: Vec<PathBuf>,
	},
	/// The view mode changed.
	ViewChanged {
		/// The explorer's stable id.
		control: usize,
		/// The new view mode.
		view: ExplorerView,
	},
}

/// Flatten a tree into visible rows, honoring the `expanded` set.
///
/// Pure and unit-tested; `children` resolves a folder's entries.
pub fn flatten_tree<F>(
	roots: &[ProjectEntry],
	expanded: &HashSet<u64>,
	children: F,
) -> Vec<(ProjectEntry, usize)>
where
	F: Fn(u64) -> Vec<ProjectEntry>,
{
	fn walk<F>(
		entries: &[ProjectEntry],
		depth: usize,
		expanded: &HashSet<u64>,
		children: &F,
		out: &mut Vec<(ProjectEntry, usize)>,
	) where
		F: Fn(u64) -> Vec<ProjectEntry>,
	{
		for entry in entries {
			out.push((entry.clone(), depth));
			if entry.is_dir && expanded.contains(&entry.id) {
				let kids = children(entry.id);
				walk(&kids, depth + 1, expanded, children, out);
			}
		}
	}

	let mut out = Vec::new();
	walk(roots, 0, expanded, &children, &mut out);
	out
}

/// A material bin with tree and icon views.
pub struct ProjectExplorer<D: ProjectDataSource> {
	control: usize,
	data: Entity<D>,
	expanded: HashSet<u64>,
	selected: Option<u64>,
	view: ExplorerView,
	focus_handle: FocusHandle,
}

impl<D: ProjectDataSource> ProjectExplorer<D> {
	/// Create an explorer over `data`.
	pub fn new(
		control: usize,
		data: Entity<D>,
		_window: &mut Window,
		cx: &mut Context<Self>,
	) -> Self {
		Self {
			control,
			data,
			expanded: HashSet::new(),
			selected: None,
			view: ExplorerView::Tree,
			focus_handle: cx.focus_handle(),
		}
	}

	/// The current view mode.
	pub fn view(&self) -> ExplorerView {
		self.view
	}

	/// Set the view mode from the host.
	pub fn set_view(&mut self, view: ExplorerView, cx: &mut Context<Self>) {
		if self.view != view {
			self.view = view;
			cx.notify();
		}
	}

	fn toggle(&mut self, id: u64, cx: &mut Context<Self>) {
		if !self.expanded.remove(&id) {
			self.expanded.insert(id);
		}
		cx.notify();
	}

	fn open(&mut self, entry: &ProjectEntry, cx: &mut Context<Self>) {
		cx.emit(ProjectExplorerEvent::OpenRequested {
			control: self.control,
			id: entry.id,
			name: entry.name.clone(),
		});
		cx.notify();
	}

	fn set_view_mode(&mut self, view: ExplorerView, cx: &mut Context<Self>) {
		if self.view != view {
			self.view = view;
			cx.emit(ProjectExplorerEvent::ViewChanged {
				control: self.control,
				view,
			});
			cx.notify();
		}
	}
}

impl<D: ProjectDataSource> EventEmitter<ProjectExplorerEvent> for ProjectExplorer<D> {}

impl<D: ProjectDataSource> Focusable for ProjectExplorer<D> {
	fn focus_handle(&self, _cx: &App) -> FocusHandle {
		self.focus_handle.clone()
	}
}

impl<D: ProjectDataSource> Render for ProjectExplorer<D> {
	fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
		let colors = cx.default_colors().clone();
		let roots = self.data.read(cx).roots();
		let selected = self.selected;
		let control = self.control;

		// Toolbar: view toggle.
		let toolbar = div()
			.flex()
			.items_center()
			.gap_1()
			.px_2()
			.py_1()
			.bg(colors.container)
			.child(toggle_button(
				"gpui-widgets-explorer-tree",
				"树",
				self.view == ExplorerView::Tree,
				&colors,
				cx.listener(|this, _event: &ClickEvent, _window, cx| {
					this.set_view_mode(ExplorerView::Tree, cx);
				}),
			))
			.child(toggle_button(
				"gpui-widgets-explorer-icons",
				"图标",
				self.view == ExplorerView::Icons,
				&colors,
				cx.listener(|this, _event: &ClickEvent, _window, cx| {
					this.set_view_mode(ExplorerView::Icons, cx);
				}),
			));

		let content = match self.view {
			ExplorerView::Tree => {
				let expanded = self.expanded.clone();
				let rows = flatten_tree(&roots, &expanded, |id| self.data.read(cx).children(id));
				let mut column = div()
					.id(ElementId::named_usize(
						"gpui-widgets-explorer-tree",
						control,
					))
					.flex()
					.flex_col()
					.py_1()
					.overflow_y_scroll();
				for (entry, depth) in rows {
					let is_selected = selected == Some(entry.id);
					let click_entry = entry.clone();
					let entry_id = entry.id;
					let mut row = div()
						.id(ElementId::named_usize(
							"gpui-widgets-explorer-entry",
							entry.id as usize,
						))
						.debug_selector(move || {
							format!("gpui-widgets-explorer-entry-{entry_id}").into()
						})
						.h(px(24.0))
						.flex()
						.items_center()
						.gap_1()
						.pl(px(8.0 + depth as f32 * 14.0))
						.pr_2()
						.cursor_pointer()
						.bg(if is_selected {
							colors.selected
						} else {
							transparent()
						})
						.text_color(if is_selected {
							colors.selected_text
						} else {
							colors.text
						})
						.on_click(cx.listener(move |this, event: &ClickEvent, _window, cx| {
							this.selected = Some(click_entry.id);
							if click_entry.is_dir {
								if event.click_count() >= 2 {
									this.open(&click_entry, cx);
								} else {
									this.toggle(click_entry.id, cx);
								}
							} else if event.click_count() >= 2 {
								this.open(&click_entry, cx);
							}
							cx.notify();
						}))
						.child(div().w(px(14.0)).child(if entry.is_dir {
							if this_expanded(&expanded, entry.id) {
								"▾"
							} else {
								"▸"
							}
						} else {
							""
						}))
						.child(div().child(entry.name.clone()));
					if !is_selected {
						row = row.hover(|style| style.bg(Hsla::from(colors.selected).opacity(0.3)));
					}
					column = column.child(row);
				}
				column
			}
			ExplorerView::Icons => {
				// A flat grid of the roots' children with thumbnails.
				let mut grid = div()
					.id(ElementId::named_usize(
						"gpui-widgets-explorer-icons",
						control,
					))
					.flex()
					.flex_wrap()
					.gap_2()
					.p_2()
					.overflow_y_scroll();
				for entry in roots
					.iter()
					.flat_map(|root| self.data.read(cx).children(root.id))
				{
					let is_selected = selected == Some(entry.id);
					let click_entry = entry.clone();
					grid = grid.child(
						div()
							.id(ElementId::named_usize(
								"gpui-widgets-explorer-icon",
								entry.id as usize,
							))
							.w(px(96.0))
							.p_1()
							.rounded_md()
							.flex()
							.flex_col()
							.items_center()
							.gap_1()
							.cursor_pointer()
							.bg(if is_selected {
								colors.selected
							} else {
								transparent()
							})
							.on_click(cx.listener(move |this, event: &ClickEvent, _window, cx| {
								this.selected = Some(click_entry.id);
								if event.click_count() >= 2 {
									this.open(&click_entry, cx);
								}
								cx.notify();
							}))
							.child(if let Some(thumbnail) = entry.thumbnail.clone() {
								img(thumbnail).w(px(72.0)).h(px(48.0)).into_any_element()
							} else {
								div()
									.w(px(72.0))
									.h(px(48.0))
									.rounded_md()
									.bg(Hsla::from(colors.selected).opacity(0.4))
									.flex()
									.items_center()
									.justify_center()
									.text_color(colors.text)
									.child(entry.name.chars().next().unwrap_or(' ').to_string())
									.into_any_element()
							})
							.child(
								div()
									.w_full()
									.text_color(colors.text)
									.child(entry.name.clone()),
							),
					);
				}
				grid
			}
		};

		div()
			.id(ElementId::named_usize("gpui-widgets-explorer", control))
			.size_full()
			.flex()
			.flex_col()
			.on_drop(
				cx.listener(|this, paths: &Arc<ExternalPaths>, _window, cx| {
					cx.emit(ProjectExplorerEvent::FileDropRequested {
						control: this.control,
						paths: paths.0.iter().cloned().collect(),
					});
					cx.notify();
				}),
			)
			.child(toolbar)
			.child(content.flex_1())
	}
}

fn this_expanded(expanded: &HashSet<u64>, id: u64) -> bool {
	expanded.contains(&id)
}

/// A small toggle button for the view switcher.
fn toggle_button(
	id: &'static str,
	label: &'static str,
	active: bool,
	colors: &gpui::colors::Colors,
	on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
	div()
		.id(id)
		.debug_selector(move || id.into())
		.px_2()
		.py_1()
		.rounded_md()
		.bg(if active {
			colors.selected
		} else {
			transparent()
		})
		.text_color(if active {
			colors.selected_text
		} else {
			colors.text
		})
		.cursor_pointer()
		.on_click(on_click)
		.child(label)
}

#[cfg(test)]
mod tests {
	use super::*;
	use gpui::{
		Entity, Modifiers, MouseButton, MouseDownEvent, TestAppContext, VisualTestContext, px, size,
	};

	#[test]
	fn flatten_tree_honors_expansion() {
		let roots = vec![
			ProjectEntry::new(1, "Footage", true),
			ProjectEntry::new(2, "Notes.md", false),
		];
		let expanded = HashSet::from([1u64]);
		let children = |id: u64| -> Vec<ProjectEntry> {
			if id == 1 {
				vec![
					ProjectEntry::new(10, "a.mov", false),
					ProjectEntry::new(11, "b.mov", false),
				]
			} else {
				Vec::new()
			}
		};
		let rows = flatten_tree(&roots, &expanded, children);
		assert_eq!(
			rows.iter().map(|(e, d)| (e.id, *d)).collect::<Vec<_>>(),
			vec![(1, 0), (10, 1), (11, 1), (2, 0)]
		);

		let collapsed = flatten_tree(&roots, &HashSet::new(), children);
		assert_eq!(
			collapsed.iter().map(|(e, _)| e.id).collect::<Vec<_>>(),
			vec![1, 2]
		);
	}

	#[test]
	fn entry_thumbnail_optional() {
		let plain = ProjectEntry::new(1, "x.mov", false);
		assert_eq!(plain.thumbnail, None);
		let with = plain.clone().with_thumbnail("thumbs/x.png");
		assert_eq!(with.thumbnail.as_deref(), Some("thumbs/x.png"));
	}

	// --- view interaction tests ---

	struct MockData;
	impl ProjectDataSource for MockData {
		fn roots(&self) -> Vec<ProjectEntry> {
			vec![
				ProjectEntry::new(1, "Footage", true),
				ProjectEntry::new(2, "Notes.md", false),
			]
		}
		fn children(&self, parent_id: u64) -> Vec<ProjectEntry> {
			if parent_id == 1 {
				vec![ProjectEntry::new(10, "a.mov", false)]
			} else {
				Vec::new()
			}
		}
	}

	struct Host {
		explorer: Entity<ProjectExplorer<MockData>>,
		events: Vec<ProjectExplorerEvent>,
	}
	impl Render for Host {
		fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
			div().size_full().child(self.explorer.clone())
		}
	}

	fn make_explorer(cx: &mut TestAppContext) -> (&'static mut VisualTestContext, Entity<Host>) {
		cx.update(|cx| cx.init_colors());
		let window = cx.open_window(size(px(300.0), px(240.0)), |window, cx| {
			let data = cx.new(|_| MockData);
			let explorer = cx.new(|cx| ProjectExplorer::new(1, data, window, cx));
			let host = Host {
				explorer,
				events: Vec::new(),
			};
			cx.subscribe(
				&host.explorer,
				|host: &mut Host,
				 _e: Entity<ProjectExplorer<MockData>>,
				 event: &ProjectExplorerEvent,
				 _cx: &mut Context<Host>| {
					host.events.push(event.clone());
				},
			)
			.detach();
			host
		});
		cx.run_until_parked();
		let host = window.root(cx).unwrap();
		let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
		(cx, host)
	}

	#[gpui::test]
	async fn clicking_a_folder_expands_it(cx: &mut TestAppContext) {
		let (cx, _host) = make_explorer(cx);
		// The first row (Footage) is at the top of the content area.
		let first = cx
			.debug_bounds("gpui-widgets-explorer-entry-1")
			.expect("first row rendered");
		cx.simulate_click(first.center(), Modifiers::none());
		cx.run_until_parked();
		cx.update(|window, cx| {
			window.draw(cx).clear();
		});

		// The child a.mov should now be visible.
		let child = cx.debug_bounds("gpui-widgets-explorer-entry-10");
		assert!(child.is_some(), "expanded folder should reveal its child");
	}

	#[gpui::test]
	async fn double_clicking_a_file_emits_open_request(cx: &mut TestAppContext) {
		let (cx, host) = make_explorer(cx);
		// Expand Footage first.
		let first = cx
			.debug_bounds("gpui-widgets-explorer-entry-1")
			.expect("first row rendered");
		cx.simulate_click(first.center(), Modifiers::none());
		cx.run_until_parked();
		cx.update(|window, cx| {
			window.draw(cx).clear();
		});

		// Double-click a.mov.
		let child = cx
			.debug_bounds("gpui-widgets-explorer-entry-10")
			.expect("child row rendered");
		let modifiers = Modifiers::none();
		cx.simulate_event(MouseDownEvent {
			position: child.center(),
			modifiers,
			button: MouseButton::Left,
			click_count: 2,
			first_mouse: false,
		});
		cx.simulate_event(gpui::MouseUpEvent {
			position: child.center(),
			modifiers,
			button: MouseButton::Left,
			click_count: 2,
		});
		cx.run_until_parked();

		let opened = cx.read(|app| {
			host.read(app)
				.events
				.iter()
				.any(|e| matches!(e, ProjectExplorerEvent::OpenRequested { id: 10, .. }))
		});
		assert!(opened, "expected an OpenRequested for a.mov");
	}

	#[gpui::test]
	async fn switching_to_icons_emits_view_changed(cx: &mut TestAppContext) {
		let (cx, host) = make_explorer(cx);
		let toggle = cx
			.debug_bounds("gpui-widgets-explorer-icons")
			.expect("icons toggle rendered");
		cx.simulate_click(toggle.center(), Modifiers::none());
		cx.run_until_parked();

		let changed = cx.read(|app| {
			host.read(app).events.iter().any(|e| {
				matches!(
					e,
					ProjectExplorerEvent::ViewChanged {
						view: ExplorerView::Icons,
						..
					}
				)
			})
		});
		assert!(changed, "expected a ViewChanged(Icons) event");
	}
}
