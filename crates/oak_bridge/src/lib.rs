//! macOS video-frame bridge for Oak: engine wgpu (Metal) textures are copied
//! GPU-to-GPU into an IOSurface-backed [`CVPixelBuffer`], which gpui's
//! `Surface` element / `window.paint_surface` samples zero-copy. A CPU
//! readback path works on any backend as a fallback.
//!
//! On non-macOS platforms this crate is empty.

#![cfg_attr(not(target_os = "macos"), allow(unused))]

#[cfg(target_os = "macos")]
pub mod surface;
