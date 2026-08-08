//! A material-bin demo: a mock project shown in the ProjectExplorer's tree
//! and icon views, printing every request event (open / view change / file
//! drop).

use gpui::{
    App, Bounds, Context, Entity, Render, Window, WindowBounds, WindowOptions, div, prelude::*,
    px, size,
};
use gpui_widgets::project_explorer::{
    ProjectDataSource, ProjectEntry, ProjectExplorer, ProjectExplorerEvent,
};

struct MockProject {
    entries: Vec<ProjectEntry>,
}

impl MockProject {
    fn demo() -> Self {
        Self {
            entries: vec![
                ProjectEntry::new(1, "Footage", true),
                ProjectEntry::new(2, "Bins", true),
                ProjectEntry::new(3, "Notes.md", false),
            ],
        }
    }
}

impl ProjectDataSource for MockProject {
    fn roots(&self) -> Vec<ProjectEntry> {
        self.entries
            .iter()
            .filter(|e| matches!(e.id, 1..=3))
            .cloned()
            .collect()
    }

    fn children(&self, parent_id: u64) -> Vec<ProjectEntry> {
        match parent_id {
            1 => vec![
                ProjectEntry::new(10, "intro.mov", false).with_thumbnail("assets/thumb.png"),
                ProjectEntry::new(11, "b-roll.mov", false),
                ProjectEntry::new(12, "interview.mov", false),
            ],
            2 => vec![
                ProjectEntry::new(20, "Selects", true),
                ProjectEntry::new(21, "Music", true),
            ],
            20 => vec![ProjectEntry::new(200, "best-take.mov", false)],
            21 => vec![
                ProjectEntry::new(210, "track-01.wav", false),
                ProjectEntry::new(211, "track-02.wav", false),
            ],
            _ => Vec::new(),
        }
    }
}

struct Example {
    explorer: Entity<ProjectExplorer<MockProject>>,
}

impl Example {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let project = cx.new(|_| MockProject::demo());
        let explorer = cx.new(|cx| ProjectExplorer::new(1, project, window, cx));
        cx.subscribe(
            &explorer,
            |_this: &mut Self,
             _e: Entity<ProjectExplorer<MockProject>>,
             event: &ProjectExplorerEvent,
             _cx| {
                println!("explorer request: {event:?}");
            },
        )
        .detach();
        Self { explorer }
    }
}

impl Render for Example {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.explorer.clone())
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        cx.init_colors();
        let bounds = Bounds::centered(None, size(px(420.0), px(560.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Example::new(window, cx)),
        )
        .expect("Failed to open window");

        cx.activate(true);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
    });
}
