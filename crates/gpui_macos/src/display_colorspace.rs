// GPUI macOS platform - GPUI is licensed under the Apache License, Version 2.0
// (see the gpui submodule's license).

//! The colorspace tagging of the window's Metal layer, for the
//! color-management coordination in `metal_renderer` (the single-mapping
//! rule: whoever maps the pixels to the display — the OS or the app — the
//! layer is tagged so the other side does not map them again).
//!
//! This lives in its own module because the `metal_renderer` build script
//! scans `metal_renderer.rs` for FFI declarations and forwards them into
//! the Metal shader header — raw extern blocks must not be added there.

use std::ffi::c_void;

use gpui::{ContentPrimaries, ContentTransfer, WindowContentColorspace};

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
	fn CGMainDisplayID() -> u32;
	fn CGDisplayCopyColorSpace(display: u32) -> *mut c_void;
	fn CGColorSpaceCreateWithName(name: *const c_void) -> *mut c_void;
	static kCGColorSpaceSRGB: *const c_void;
	static kCGColorSpaceDisplayP3: *const c_void;
	static kCGColorSpaceITUR_2020: *const c_void;
	// The BT.2100 constants exist only on macOS 10.15+ (the deployment
	// target); on older systems they resolve to nil and creation fails.
	static kCGColorSpaceITUR_2100_PQ: *const c_void;
	static kCGColorSpaceITUR_2100_HLG: *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
	fn CFRelease(obj: *const c_void);
}

/// Run `f` with the colorspace of `display_id` as a raw `CGColorSpaceRef`
/// (caller's responsibility to keep it alive for the duration of the call;
/// released here afterwards — receivers such as the Metal layer retain it).
/// Returns whether `f` ran; `false` when the display has no colorspace
/// (headless), in which case the caller keeps whatever tag the layer had.
pub fn with_display_colorspace(display_id: u32, f: impl FnOnce(*mut c_void)) -> bool {
	unsafe {
		let space = CGDisplayCopyColorSpace(display_id);
		if space.is_null() {
			return false;
		}
		f(space);
		CFRelease(space);
		true
	}
}

/// The main display's colorspace (kept for callers without a specific
/// screen context).
pub fn with_main_display_colorspace(f: impl FnOnce(*mut c_void)) {
	unsafe {
		let display = CGMainDisplayID();
		with_display_colorspace(display, f);
	}
}

/// The main display's `CGDirectDisplayID` (fallback when a window has no
/// screen to ask for its own).
pub fn main_display_id() -> u32 {
	unsafe { CGMainDisplayID() }
}

/// Run `f` with the sRGB colorspace as a raw `CGColorSpaceRef` (the
/// declaration that the layer's content is colorimetric sRGB, so
/// ColorSync performs the display mapping). Skips `f` when the colorspace
/// cannot be created.
pub fn with_srgb_colorspace(f: impl FnOnce(*mut c_void)) {
	unsafe {
		let space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
		if space.is_null() {
			return;
		}
		f(space);
		CFRelease(space);
	}
}

/// Run `f` with the `CGColorSpaceRef` matching the window's declared
/// content colorspace — the tag says what the layer's pixels *are*, so
/// ColorSync performs the one mapping to the display (the single-mapping
/// rule in this module's docs). Falls back toward sRGB and skips `f` only
/// if nothing could be created.
pub fn with_content_colorspace(spec: WindowContentColorspace, f: impl FnOnce(*mut c_void)) {
	unsafe {
		let name = match spec.transfer {
			ContentTransfer::Srgb | ContentTransfer::Gamma22 => match spec.primaries {
				ContentPrimaries::Srgb => kCGColorSpaceSRGB,
				ContentPrimaries::DisplayP3 => kCGColorSpaceDisplayP3,
				ContentPrimaries::Bt2020 => kCGColorSpaceITUR_2020,
			},
			// The ITU-R BT.2100 constants exist only on macOS 10.15+.
			ContentTransfer::Pq => kCGColorSpaceITUR_2100_PQ,
			ContentTransfer::Hlg => kCGColorSpaceITUR_2100_HLG,
		};
		let space = CGColorSpaceCreateWithName(name);
		if !space.is_null() {
			f(space);
			CFRelease(space);
			return;
		}
		// Creation failed (a BT.2100 declaration on macOS < 10.15, or an
		// unusual named-space miss). Fall back — HLG to PQ, everything else
		// to sRGB — so the layer keeps a valid colorspace declaration.
		log::warn!(
			"no CGColorSpace for content colorspace {:?}, falling back",
			spec
		);
		let fallback = if matches!(spec.transfer, ContentTransfer::Hlg) {
			kCGColorSpaceITUR_2100_PQ
		} else {
			kCGColorSpaceSRGB
		};
		let space = CGColorSpaceCreateWithName(fallback);
		if space.is_null() {
			return;
		}
		f(space);
		CFRelease(space);
	}
}
