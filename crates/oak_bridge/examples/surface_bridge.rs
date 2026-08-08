//! FPS demo for the surface bridge: generate a moving test pattern on a wgpu
//! texture, bridge it to an IOSurface-backed CVPixelBuffer, and paint it into
//! a gpui window with the `Surface` element.
//!
//! macOS + wgpu Metal backend only; run with
//! `cargo run -p oak_bridge --example surface_bridge --features demo`.

#![cfg(target_os = "macos")]

use core_video::pixel_buffer::CVPixelBuffer;
use gpui::{App, Bounds, Context, Render, Window, WindowBounds, WindowOptions, div, prelude::*, px, size, surface};
use oak_bridge::surface::{SurfaceBridge, SurfaceFormat};
use std::sync::Arc;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;

struct Demo {
    queue: wgpu::Queue,
    bridge: SurfaceBridge,
    src_texture: wgpu::Texture,
    frame: u64,
    pixel_buffer: Option<CVPixelBuffer>,
    frames: u64,
    fps_clock: std::time::Instant,
}

impl Demo {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .expect("no Metal adapter available (CI environments skip this demo)");
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("oak-bridge-demo"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::default(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            },
        ))
        .expect("failed to create wgpu device");
        let device = Arc::new(device);

        // wgpu's Metal backend uses the system default device unless told
        // otherwise, so the demo reuses it for the bridge.
        let metal_device = metal::Device::system_default().expect("no Metal device");
        let bridge = SurfaceBridge::new(&metal_device, device.clone(), WIDTH, HEIGHT, SurfaceFormat::Bgra8Unorm)
            .expect("failed to create surface bridge");

        let src_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("demo-source"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SurfaceFormat::Bgra8Unorm.wgpu(),
            usage: wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        // Kick off the frame loop on the main thread.
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
            queue,
            bridge,
            src_texture,
            frame: 0,
            pixel_buffer: None,
            frames: 0,
            fps_clock: std::time::Instant::now(),
        }
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        // Generate a moving color-bar + gradient test pattern on the CPU and
        // upload it (in the real engine this texture comes from a render pass).
        let mut bytes = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let index = ((y * WIDTH + x) * 4) as usize;
                let t = self.frame as f32 / 60.0;
                let hue = (x as f32 / WIDTH as f32 + t).fract();
                let stripe = if (x / 128) % 2 == 0 { 1.0 } else { 0.7 };
                let fade = (y as f32 / HEIGHT as f32) * 0.6 + 0.2;
                // BGR(A) byte order.
                bytes[index] = (255.0 * stripe * fade * (1.0 - hue)).round() as u8;
                bytes[index + 1] = (255.0 * stripe * fade * hue).round() as u8;
                bytes[index + 2] = (255.0 * stripe * fade * (0.5 + 0.5 * (t * 2.0).sin())).round() as u8;
                bytes[index + 3] = 255;
            }
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.src_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 4),
                rows_per_image: None,
            },
            self.src_texture.size(),
        );

        // Bridge to an IOSurface-backed CVPixelBuffer (GPU-to-GPU).
        match self.bridge.blit_frame(&self.src_texture) {
            Ok(pixel_buffer) => {
                self.pixel_buffer = Some(pixel_buffer);
            }
            Err(error) => {
                eprintln!("blit failed: {error}");
            }
        }
        self.frame += 1;
        self.frames += 1;

        // FPS meter, once per second.
        if self.fps_clock.elapsed() >= std::time::Duration::from_secs(1) {
            println!("{:.0} fps ({}x{})", self.frames as f64 / self.fps_clock.elapsed().as_secs_f64(), WIDTH, HEIGHT);
            self.frames = 0;
            self.fps_clock = std::time::Instant::now();
        }

        cx.notify();
    }
}

impl Render for Demo {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(pixel_buffer) = self.pixel_buffer.clone() {
            div().size_full().child(surface(pixel_buffer))
        } else {
            div().size_full().child("waiting for first frame…")
        }
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
            |window, cx| cx.new(|cx| Demo::new(window, cx)),
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
