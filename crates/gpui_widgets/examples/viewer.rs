//! A viewer demo: a mock playback clock drives the transport bar and (on
//! macOS) a generated test frame fills the picture area every tick.
//!
//! Run with `cargo run -p gpui_widgets --example viewer`.

use gpui::{
    App, Bounds, Context, Entity, Render, Window, WindowBounds, WindowOptions, div, prelude::*,
    px, size,
};
use gpui::timeline::{Frame, FrameRate};
use gpui_widgets::viewer::{PlaybackClock, ViewerEvent, ViewerWidget};

#[cfg(target_os = "macos")]
mod frame_gen {
    use core_video::pixel_buffer::CVPixelBuffer;
    use std::ffi::c_void;

    const WIDTH: u32 = 1280;
    const HEIGHT: u32 = 720;

    /// Generates cheap test frames (a moving hue band) into CVPixelBuffers.
    pub struct FrameGen {
        frame: u64,
    }

    impl FrameGen {
        pub fn new() -> Self {
            Self { frame: 0 }
        }

        pub fn next(&mut self) -> CVPixelBuffer {
            let mut bytes = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    let i = ((y * WIDTH + x) * 4) as usize;
                    let t = self.frame as f32 / 60.0;
                    let band = if (x as f32 / 128.0 + t * 8.0).fract() < 0.5 {
                        1.0
                    } else {
                        0.35
                    };
                    bytes[i] = (255.0f32 * band).round() as u8;
                    bytes[i + 1] = (255.0f32 * band * (y as f32 / HEIGHT as f32)).round() as u8;
                    bytes[i + 2] = (255.0f32 * band * (1.0 - y as f32 / HEIGHT as f32)).round() as u8;
                    bytes[i + 3] = 255;
                }
            }
            let callback_ref: Box<Vec<u8>> = Box::new(bytes);
            let release_con = Box::into_raw(callback_ref) as *mut c_void;
            self.frame += 1;
            unsafe {
                CVPixelBuffer::new_with_bytes(
                    0x42475241, // kCVPixelFormatType_32BGRA
                    WIDTH as usize,
                    HEIGHT as usize,
                    release_con as *mut c_void,
                    (WIDTH * 4) as usize,
                    free_bytes,
                    release_con,
                    None,
                )
            }
            .expect("failed to create test pixel buffer")
        }

    }

    extern "C" fn free_bytes(release_ref_con: *mut c_void, _base: *const *const c_void) {
        if !release_ref_con.is_null() {
            unsafe {
                drop(Box::from_raw(release_ref_con as *mut Vec<u8>));
            }
        }
    }
}

struct MockClock {
    frame: Frame,
    playing: bool,
    rate: FrameRate,
}

impl PlaybackClock for MockClock {
    fn current_frame(&self) -> Frame {
        self.frame
    }
    fn is_playing(&self) -> bool {
        self.playing
    }
    fn frame_rate(&self) -> FrameRate {
        self.rate
    }
}

struct Example {
    clock: Entity<MockClock>,
    viewer: Entity<ViewerWidget<MockClock>>,
    #[cfg(target_os = "macos")]
    frame_gen: frame_gen::FrameGen,
}

impl Example {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let clock = cx.new(|_| MockClock {
            frame: Frame(0),
            playing: true,
            rate: FrameRate::new(30, 1),
        });
        let viewer = cx.new(|cx| ViewerWidget::new(1, clock.clone(), window, cx));
        cx.subscribe(
            &viewer,
            |_this: &mut Self, _v: Entity<ViewerWidget<MockClock>>, event: &ViewerEvent, _cx| {
                println!("viewer request: {event:?}");
            },
        )
        .detach();

        let this = cx.weak_entity();
        window.spawn(cx, async move |cx: &mut gpui::AsyncWindowContext| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(16))
                    .await;
                let _ = cx.update(|_window, app| {
                    if let Some(this) = this.upgrade() {
                        this.update(app, |this, cx| this.tick(cx));
                    }
                });
            }
        })
        .detach();

        Self {
            clock,
            viewer,
            #[cfg(target_os = "macos")]
            frame_gen: frame_gen::FrameGen::new(),
        }
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        // Simulate the engine: advance the clock while playing.
        self.clock.update(cx, |clock, _cx| {
            if clock.playing {
                clock.frame = Frame((clock.frame.0 + 1) % (30 * 60 * 5));
            }
        });
        // Feed a fresh frame (macOS only).
        #[cfg(target_os = "macos")]
        {
            let buffer = self.frame_gen.next();
            self.viewer
                .update(cx, |viewer, cx| viewer.set_frame_source(Some(buffer.into()), cx));
        }
        cx.notify();
    }
}

impl Render for Example {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.viewer.clone())
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        cx.init_colors();
        let bounds = Bounds::centered(None, size(px(960.0), px(540.0)), cx);
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
