// GPUI macOS platform - GPUI is licensed under the Apache License, Version 2.0
// (see the gpui submodule's license).

//! The main display's colorspace as a raw `CGColorSpaceRef`, for the
//! color-management coordination in `metal_renderer` (when the app
//! self-manages the display transform, the CAMetalLayer is tagged with the
//! display's colorspace so ColorSync passes the pixels through).
//!
//! This lives in its own module because the `metal_renderer` build script
//! scans `metal_renderer.rs` for FFI declarations and forwards them into
//! the Metal shader header — raw extern blocks must not be added there.

use std::ffi::c_void;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
	fn CGMainDisplayID() -> u32;
	fn CGDisplayCopyColorSpace(display: u32) -> *mut c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
	fn CFRelease(obj: *const c_void);
}

/// The main display's colorspace as a raw pointer (caller's
/// responsibility to keep it alive for the msg_send call; released here
/// after the layer call returns — the layer retains it). `None` when the
/// display has no colorspace (headless).
pub fn with_main_display_colorspace(f: impl FnOnce(*mut c_void)) {
	unsafe {
		let space = CGDisplayCopyColorSpace(CGMainDisplayID());
		if space.is_null() {
			return;
		}
		f(space);
		CFRelease(space);
	}
}
