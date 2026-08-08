//! Demo of the `gpui::timeline` video-editing timeline widget.
//!
//! This is the intended-usage sketch: a mock [`TimelineDataSource`] with two
//! video and two audio tracks carrying a handful of static clips, a
//! [`TimelineView`] placed in a window, and an event subscription that logs
//! the edit requests the widget emits.
//!
//! In a real host (Oak), the `match` arm in `TimelineExample::new` is where
//! each [`TimelineEvent`] becomes an undoable engine command, followed by a
//! `cx.notify()` on the model entity.
//!
//! NOTE: the timeline's rendering and interaction internals are still
//! `todo!()`; this example compiles and shows the wiring, not a usable UI.

// The modules under demo are skeletons whose bodies are `todo!()` by design.
#![allow(clippy::todo)]

use gpui::{
    App, Bounds, Context, Entity, Pixels, Render, SharedString, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, size,
};
use gpui::timeline::{
    ClipData, ClipId, Frame, FrameRange, FrameRate, TimelineDataSource, TimelineEvent,
    TimelineView, TrackData, TrackKind,
};

#[path = "../shared/prelude.rs"]
mod example_prelude;

// --- mock model ------------------------------------------------------------

struct MockClip {
    id: ClipId,
    range: FrameRange,
    media_in: Frame,
    label: SharedString,
}

impl ClipData for MockClip {
    fn id(&self) -> ClipId {
        self.id
    }

    fn range(&self) -> FrameRange {
        self.range
    }

    fn media_in(&self) -> Frame {
        self.media_in
    }

    fn label(&self) -> SharedString {
        self.label.clone()
    }
}

struct MockTrack {
    kind: TrackKind,
    name: SharedString,
    height: Pixels,
    clips: Vec<MockClip>,
}

impl TrackData for MockTrack {
    type Clip = MockClip;

    fn kind(&self) -> TrackKind {
        self.kind
    }

    fn name(&self) -> SharedString {
        self.name.clone()
    }

    fn height(&self) -> Pixels {
        self.height
    }

    fn clips(&self) -> &[Self::Clip] {
        &self.clips
    }
}

struct MockSequence {
    tracks: Vec<MockTrack>,
}

impl MockSequence {
    fn demo() -> Self {
        let clip = |id: u64, start: i64, end: i64, label: &str| MockClip {
            id: ClipId(id),
            range: FrameRange::new(Frame(start), Frame(end)),
            media_in: Frame::ZERO,
            label: label.into(),
        };
        MockSequence {
            tracks: vec![
                MockTrack {
                    kind: TrackKind::Video,
                    name: "V1".into(),
                    height: px(64.),
                    clips: vec![clip(1, 0, 240, "opening.mov"), clip(2, 240, 600, "b-roll.mp4")],
                },
                MockTrack {
                    kind: TrackKind::Video,
                    name: "V2".into(),
                    height: px(64.),
                    clips: vec![clip(3, 120, 300, "title.mov")],
                },
                MockTrack {
                    kind: TrackKind::Audio,
                    name: "A1".into(),
                    height: px(48.),
                    clips: vec![clip(4, 0, 600, "dialog.wav")],
                },
                MockTrack {
                    kind: TrackKind::Audio,
                    name: "A2".into(),
                    height: px(48.),
                    clips: vec![clip(5, 0, 480, "score.flac")],
                },
            ],
        }
    }
}

impl TimelineDataSource for MockSequence {
    type Track = MockTrack;

    fn frame_rate(&self) -> FrameRate {
        FrameRate::NTSC_2997
    }

    fn sequence_length(&self) -> Frame {
        Frame(600)
    }

    fn track_count(&self) -> usize {
        self.tracks.len()
    }

    fn track(&self, index: usize) -> Option<Self::Track> {
        // A real host returns a lightweight snapshot; the mock simply
        // reports the track's existence. Returning an owned value here is
        // what the trait requires, so the mock clones its clips.
        self.tracks.get(index).map(|t| MockTrack {
            kind: t.kind,
            name: t.name.clone(),
            height: t.height,
            clips: t
                .clips
                .iter()
                .map(|c| MockClip {
                    id: c.id,
                    range: c.range,
                    media_in: c.media_in,
                    label: c.label.clone(),
                })
                .collect(),
        })
    }
}

// --- the example view ------------------------------------------------------

struct TimelineExample {
    timeline: Entity<TimelineView<MockSequence>>,
}

impl TimelineExample {
    fn new(model: Entity<MockSequence>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let timeline = cx.new(|cx| TimelineView::new(model, window, cx).zoom(2.0));
        cx.subscribe(
            &timeline,
            |_this, _timeline, event: &TimelineEvent, _cx| {
                // In Oak, each request becomes an undoable engine command
                // here, followed by `model.update(cx, |_, cx| cx.notify())`.
                println!("timeline edit request: {event:?}");
            },
        )
        .detach();
        TimelineExample { timeline }
    }
}

impl Render for TimelineExample {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.timeline.clone())
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1000.), px(480.)), cx);
        let model = cx.new(|_cx| MockSequence::demo());
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| TimelineExample::new(model, window, cx)),
        )
        .expect("Failed to open window");

        example_prelude::init_example(cx, "Timeline");
    });
}
