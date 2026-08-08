//! The viewer widget: a fullscreen preview with a transport bar.
//!
//! The picture comes from a [`SurfaceSource`] (in Oak, the W3 bridge's
//! IOSurface-backed pixel buffer); the playhead position is polled from the
//! host's [`PlaybackClock`] on a ~60 Hz timer, and every transport action is
//! emitted as a [`ViewerEvent`] request (the engine applies it and the clock
//! reflects it). Timecode formatting reuses `gpui::timeline::time`.

pub mod clock;
pub mod transport;

pub use clock::*;
pub use transport::*;

use gpui::timeline::{FrameRate, TimeDisplay, format_timecode};
use gpui::{
    App, AsyncWindowContext, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    ObjectFit, Render, SurfaceSource, Window, colors::DefaultColors, div, prelude::*, px, surface,
};

/// A request emitted by the viewer.
#[derive(Debug, Clone, PartialEq)]
pub enum ViewerEvent {
    /// Start playback.
    PlayRequested {
        /// The viewer's stable id.
        control: usize,
    },
    /// Pause playback.
    PauseRequested {
        /// The viewer's stable id.
        control: usize,
    },
    /// Step the playhead by `delta` frames.
    StepRequested {
        /// The viewer's stable id.
        control: usize,
        /// Frames to step (negative steps backward).
        delta: i64,
    },
    /// Set the loop-in point at the playhead.
    InPointRequested {
        /// The viewer's stable id.
        control: usize,
    },
    /// Set the loop-out point at the playhead.
    OutPointRequested {
        /// The viewer's stable id.
        control: usize,
    },
    /// Clear the loop range.
    ClearRangeRequested {
        /// The viewer's stable id.
        control: usize,
    },
    /// Toggle the safe-frame overlay.
    ToggleSafeFramesRequested {
        /// The viewer's stable id.
        control: usize,
    },
    /// Toggle the zoom (contain vs cover).
    ToggleZoomRequested {
        /// The viewer's stable id.
        control: usize,
    },
}

/// The viewer widget.
pub struct ViewerWidget<C: PlaybackClock> {
    control: usize,
    clock: Entity<C>,
    frame_rate: FrameRate,
    transport: TransportState,
    frame_source: Option<SurfaceSource>,
    focus_handle: FocusHandle,
    show_safe_frames: bool,
    zoom: bool,
}

impl<C: PlaybackClock> ViewerWidget<C> {
    /// Create a viewer driven by `clock`.
    pub fn new(
        control: usize,
        clock: Entity<C>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let frame_rate = clock.read(cx).frame_rate();

        // Poll the engine clock on a timer and reflect it locally.
        let this = cx.weak_entity();
        window.spawn(cx, async move |cx: &mut AsyncWindowContext| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(16))
                    .await;
                let _ = cx.update(|_window, app| {
                    if let Some(this) = this.upgrade() {
                        this.update(app, |this, cx| this.poll_clock(cx));
                    }
                });
            }
        })
        .detach();

        Self {
            control,
            clock,
            frame_rate,
            transport: TransportState::new(),
            frame_source: None,
            focus_handle: cx.focus_handle(),
            show_safe_frames: false,
            zoom: false,
        }
    }

    /// The current transport state.
    pub fn transport(&self) -> TransportState {
        self.transport
    }

    /// Set the picture source (the bridge's pixel buffer) and repaint.
    pub fn set_frame_source(&mut self, source: Option<SurfaceSource>, cx: &mut Context<Self>) {
        self.frame_source = source;
        cx.notify();
    }

    fn poll_clock(&mut self, cx: &mut Context<Self>) {
        let clock = self.clock.read(cx);
        let frame = clock.current_frame();
        let playing = clock.is_playing();
        if frame != self.transport.frame || playing != self.transport.playing {
            self.transport.frame = frame;
            self.transport.playing = playing;
            cx.notify();
        }
    }

    fn emit(&mut self, event: ViewerEvent, cx: &mut Context<Self>) {
        cx.emit(event);
        cx.notify();
    }
}

impl<C: PlaybackClock> EventEmitter<ViewerEvent> for ViewerWidget<C> {}

impl<C: PlaybackClock> Focusable for ViewerWidget<C> {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl<C: PlaybackClock> Render for ViewerWidget<C> {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors().clone();
        let timecode = format_timecode(
            self.transport.frame,
            self.frame_rate,
            TimeDisplay::Timecode,
        );

        // The picture area: surface (or placeholder), safe frames and zoom.
        let mut picture = div()
            .id("gpui-widgets-viewer-picture")
            .flex_1()
            .relative()
            .bg(gpui::Hsla {
                h: 0.0,
                s: 0.0,
                l: 0.0,
                a: 1.0,
            });

        if let Some(source) = &self.frame_source {
            let fit = if self.zoom { ObjectFit::Cover } else { ObjectFit::Contain };
            picture = picture.child(
                surface(source.clone())
                    .size_full()
                    .object_fit(fit),
            );
        } else {
            picture = picture.child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(colors.disabled)
                    .child("No frame source"),
            );
        }

        if self.show_safe_frames {
            picture = picture.child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top_0()
                    .bottom_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .w(px(560.0))
                            .h(px(315.0))
                            .border_1()
                            .border_color(colors.selected),
                    ),
            );
        }

        // Transport bar.
        let playing = self.transport.playing;
        let play_label = if playing { "⏸" } else { "▶" };
        let transport_bar = div()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .bg(colors.container)
            .child(button(
                "gpui-widgets-viewer-in",
                "⏮",
                cx.listener(|this, _event: &ClickEvent, _window, cx| {
                    this.emit(ViewerEvent::InPointRequested { control: this.control }, cx);
                }),
            ))
            .child(button(
                "gpui-widgets-viewer-step-back",
                "⏪",
                cx.listener(|this, _event: &ClickEvent, _window, cx| {
                    this.emit(
                        ViewerEvent::StepRequested {
                            control: this.control,
                            delta: -1,
                        },
                        cx,
                    );
                }),
            ))
            .child(button(
                "gpui-widgets-viewer-play",
                play_label,
                cx.listener(|this, _event: &ClickEvent, _window, cx| {
                    let event = if this.transport.playing {
                        ViewerEvent::PauseRequested { control: this.control }
                    } else {
                        ViewerEvent::PlayRequested { control: this.control }
                    };
                    this.emit(event, cx);
                }),
            ))
            .child(button(
                "gpui-widgets-viewer-step-forward",
                "⏩",
                cx.listener(|this, _event: &ClickEvent, _window, cx| {
                    this.emit(
                        ViewerEvent::StepRequested {
                            control: this.control,
                            delta: 1,
                        },
                        cx,
                    );
                }),
            ))
            .child(button(
                "gpui-widgets-viewer-out",
                "⏭",
                cx.listener(|this, _event: &ClickEvent, _window, cx| {
                    this.emit(ViewerEvent::OutPointRequested { control: this.control }, cx);
                }),
            ))
            .child(button(
                "gpui-widgets-viewer-clear-range",
                "✕",
                cx.listener(|this, _event: &ClickEvent, _window, cx| {
                    this.emit(ViewerEvent::ClearRangeRequested { control: this.control }, cx);
                }),
            ))
            .child(
                div()
                    .px_2()
                    .text_color(colors.text)
                    .child(timecode),
            )
            .child(
                div().flex_1(),
            )
            .child(button(
                "gpui-widgets-viewer-safe",
                "安全框",
                cx.listener(|this, _event: &ClickEvent, _window, cx| {
                    this.show_safe_frames = !this.show_safe_frames;
                    this.emit(
                        ViewerEvent::ToggleSafeFramesRequested { control: this.control },
                        cx,
                    );
                }),
            ))
            .child(button(
                "gpui-widgets-viewer-zoom",
                "缩放",
                cx.listener(|this, _event: &ClickEvent, _window, cx| {
                    this.zoom = !this.zoom;
                    this.emit(ViewerEvent::ToggleZoomRequested { control: this.control }, cx);
                }),
            ));

        div().size_full().flex().flex_col().child(picture).child(transport_bar)
    }
}

/// A small labeled button.
fn button(
    id: &'static str,
    label: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .px_2()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .hover(|style| style.bg(gpui::colors::Colors::dark().selected))
        .on_click(on_click)
        .child(label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::timeline::{Frame, FrameRate};
    use gpui::{Modifiers, TestAppContext, VisualTestContext, px, size};

    struct MockClock {
        frame: Frame,
        playing: bool,
    }
    impl PlaybackClock for MockClock {
        fn current_frame(&self) -> Frame {
            self.frame
        }
        fn is_playing(&self) -> bool {
            self.playing
        }
        fn frame_rate(&self) -> FrameRate {
            FrameRate::new(30, 1)
        }
    }

    #[gpui::test]
    async fn play_button_emits_play_request(cx: &mut TestAppContext) {
        struct Host {
            viewer: Entity<ViewerWidget<MockClock>>,
            events: Vec<ViewerEvent>,
        }
        impl Render for Host {
            fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
                div().size_full().child(self.viewer.clone())
            }
        }

        cx.update(|cx| cx.init_colors());
        let window = cx.open_window(size(px(640.0), px(420.0)), |window, cx| {
            let clock = cx.new(|_| MockClock {
                frame: Frame(0),
                playing: false,
            });
            let viewer = cx.new(|cx| ViewerWidget::new(1, clock, window, cx));
            let host = Host {
                viewer,
                events: Vec::new(),
            };
            cx.subscribe(
                &host.viewer,
                |host: &mut Host,
                 _v: Entity<ViewerWidget<MockClock>>,
                 event: &ViewerEvent,
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
        // The play button sits on the left of the transport bar at the bottom.
        let play = cx
            .debug_bounds("gpui-widgets-viewer-play")
            .expect("play button rendered");
        cx.simulate_click(play.center(), Modifiers::none());
        cx.run_until_parked();

        let requested = cx.read(|app| {
            host.read(app).events.iter().any(|e| {
                matches!(e, ViewerEvent::PlayRequested { control: 1 })
            })
        });
        assert!(requested, "expected a PlayRequested event");
    }

    #[test]
    fn timecode_formatting_reuses_timeline() {
        let frame = Frame(3000);
        let text = format_timecode(frame, FrameRate::new(30, 1), TimeDisplay::Timecode);
        assert_eq!(text, "00:01:40:00");
    }
}
