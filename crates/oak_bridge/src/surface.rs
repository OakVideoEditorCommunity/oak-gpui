//! The macOS surface bridge (see crate root for the full picture).

#![allow(clippy::missing_safety_doc)]

use anyhow::{Result, anyhow, ensure};
use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use core_video::image_buffer::CVImageBufferRef;
use core_video::pixel_buffer::{
    CVPixelBuffer, kCVPixelBufferIOSurfacePropertiesKey, kCVPixelBufferMetalCompatibilityKey,
};
use media::core_video::CVMetalTextureCache;
use metal::{
    CommandQueue, MTLDevice, MTLOrigin, MTLPixelFormat, MTLSize, TextureRef,
    foreign_types::{ForeignType, ForeignTypeRef},
};
use std::ffi::c_void;
use std::sync::Arc;

/// The pixel formats the bridge can carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceFormat {
    /// 8-bit BGRA (the common video format).
    Bgra8Unorm,
    /// 16-bit float RGBA (HDR frames).
    Rgba16Float,
}

impl SurfaceFormat {
    /// The wgpu format for the engine's texture.
    pub fn wgpu(self) -> wgpu::TextureFormat {
        match self {
            SurfaceFormat::Bgra8Unorm => wgpu::TextureFormat::Bgra8Unorm,
            SurfaceFormat::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
        }
    }

    /// The Metal format used when aliasing the CVPixelBuffer.
    fn metal(self) -> MTLPixelFormat {
        match self {
            SurfaceFormat::Bgra8Unorm => MTLPixelFormat::BGRA8Unorm,
            SurfaceFormat::Rgba16Float => MTLPixelFormat::RGBA16Float,
        }
    }

    /// The CoreVideo pixel format (`OSType`).
    fn ostype(self) -> u32 {
        match self {
            // kCVPixelFormatType_32BGRA
            SurfaceFormat::Bgra8Unorm => 0x42475241,
            // kCVPixelFormatType_64RGBAHalf
            SurfaceFormat::Rgba16Float => 0x000000b4,
        }
    }
}

/// Bridges engine wgpu textures to IOSurface-backed `CVPixelBuffer`s.
///
/// `blit_frame` performs a GPU-to-GPU copy (no CPU round trip) into a reused
/// IOSurface-backed pixel buffer and hands it back for `window.paint_surface`.
/// `readback_frame` is the any-backend CPU fallback.
pub struct SurfaceBridge {
    texture_cache: CVMetalTextureCache,
    command_queue: CommandQueue,
    wgpu_device: Arc<wgpu::Device>,
    width: u32,
    height: u32,
    format: SurfaceFormat,
    /// The reused IOSurface-backed pixel buffer.
    pixel_buffer: Option<CVPixelBuffer>,
    /// The pixel buffer's Metal alias, kept alive for the blit.
    target: Option<media::core_video::CVMetalTexture>,
    /// A CPU copy for the readback path.
    cpu_bytes: Option<Box<[u8]>>,
    /// The readback staging buffer between [`Self::stage_readback`] and
    /// [`Self::finish_readback`].
    staging_buffer: Option<wgpu::Buffer>,
    readback_bytes_per_row: u32,
}

impl SurfaceBridge {
    /// Create a bridge on the given Metal device and wgpu device.
    ///
    /// `metal_device` must be the same device `wgpu_device` was created on.
    pub fn new(
        metal_device: &metal::Device,
        wgpu_device: Arc<wgpu::Device>,
        width: u32,
        height: u32,
        format: SurfaceFormat,
    ) -> Result<Self> {
        ensure!(width > 0 && height > 0, "surface must have positive size");
        let texture_cache =
            unsafe { CVMetalTextureCache::new(metal_device.as_ptr() as *mut MTLDevice) }?;
        let command_queue = metal_device.new_command_queue();
        Ok(Self {
            texture_cache,
            command_queue,
            wgpu_device,
            width,
            height,
            format,
            pixel_buffer: None,
            target: None,
            cpu_bytes: None,
            staging_buffer: None,
            readback_bytes_per_row: 0,
        })
    }

    /// The frame size.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Create (or reuse) the IOSurface-backed pixel buffer and its Metal
    /// alias, so `blit_frame` can copy into it.
    fn ensure_target(&mut self) -> Result<(CVPixelBuffer, &TextureRef)> {
        if let (Some(pb), Some(_)) = (&self.pixel_buffer, &self.target) {
            return Ok((pb.clone(), self.target.as_ref().unwrap().as_texture_ref()));
        }

        let pixel_buffer = create_iosurface_pixel_buffer(self.width, self.height, self.format)?;

        let metal_texture = unsafe {
            self.texture_cache.create_texture_from_image(
                pixel_buffer.as_concrete_TypeRef() as CVImageBufferRef,
                std::ptr::null(),
                self.format.metal(),
                self.width as usize,
                self.height as usize,
                0,
            )?
        };
        self.pixel_buffer = Some(pixel_buffer.clone());
        self.target = Some(metal_texture);
        Ok((pixel_buffer, self.target.as_ref().unwrap().as_texture_ref()))
    }

    /// Copy `src` into the IOSurface-backed pixel buffer (GPU-to-GPU, no CPU
    /// round trip) and return the buffer for display.
    ///
    /// The engine must render `src` on the same device this bridge was
    /// created with, with the same format and size, and must have submitted
    /// its work: this method waits for all of the wgpu device's submitted
    /// work to finish before issuing the Metal blit (synchronous but
    /// correct; the CPU readback is the fallback if this is too slow).
    pub fn blit_frame(&mut self, src: &wgpu::Texture) -> Result<CVPixelBuffer> {
        ensure!(
            src.size() == wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            } && src.format() == self.format.wgpu(),
            "engine texture must match the bridge format and size"
        );
        // Wait for the engine's render pass (submitted by the host on the
        // wgpu queue) to finish before reading it from our own queue.
        let _ = self.wgpu_device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });

        // Copy the queue and size out before `ensure_target` so the returned
        // `target` borrow (which lives as long as `self`) does not conflict
        // with field access below.
        let queue = self.command_queue.clone();
        let (width, height) = (self.width, self.height);

        let (pixel_buffer, target) = self.ensure_target()?;

        // The engine's texture as a Metal texture.
        let hal_texture = unsafe {
            src.as_hal::<wgpu::hal::api::Metal>()
                .expect("bridge requires the wgpu Metal backend")
        };
        let hal_texture = &*hal_texture;
        // The hal texture's first field is `raw: Retained<ProtocolObject<dyn
        // MTLTexture>>` at offset 0 (repr(Rust) keeps field order); read its
        // first word to recover the raw MTLTexture pointer. (The
        // ProtocolObject itself is a ZST, so it cannot be dereferenced.)
        // The hal texture layout (verified empirically against
        // wgpu-hal 29.0.4's Metal backend) places the MTLTexture pointer at
        // offset 8, after a small enum tag at offset 0. This is fragile by
        // nature; the CPU readback path is the robust alternative.
        let base = hal_texture as *const _ as *const u8;
        let obj_ptr = unsafe { *(base.add(8) as *const *const c_void) };
        let source = unsafe { TextureRef::from_ptr(obj_ptr as *mut metal::MTLTexture) };

        let command_buffer = queue.new_command_buffer();
        let encoder = command_buffer.new_blit_command_encoder();
        encoder.copy_from_texture(
            &source,
            0,
            0,
            MTLOrigin { x: 0, y: 0, z: 0 },
            MTLSize::new(width as u64, height as u64, 1),
            target,
            0,
            0,
            MTLOrigin { x: 0, y: 0, z: 0 },
        );
        encoder.end_encoding();
        command_buffer.commit();
        command_buffer.wait_until_completed();
        Ok(pixel_buffer)
    }

    /// CPU readback fallback, phase 1: copy `src` into a staging buffer.
    ///
    /// After calling this, submit the encoder, then call
    /// [`Self::finish_readback`] to map and wrap the bytes. Works with any
    /// backend; kept as a fallback because it round-trips through the CPU.
    pub fn stage_readback(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        src: &wgpu::Texture,
    ) -> Result<()> {
        let bytes_per_row = align_to_64(self.width * self.format.bytes_per_pixel());
        let total = bytes_per_row as u64 * self.height as u64;
        self.staging_buffer = Some(self.wgpu_device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("oak-bridge-readback"),
            size: total,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        }));
        let buffer = self.staging_buffer.as_ref().unwrap();
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: src,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: None,
                },
            },
            src.size(),
        );
        self.readback_bytes_per_row = bytes_per_row;
        Ok(())
    }

    /// CPU readback fallback, phase 2: map the staged buffer (the encoder
    /// from [`Self::stage_readback`] must already be submitted) and wrap the
    /// bytes in a `CVPixelBuffer`.
    pub fn finish_readback(&mut self) -> Result<CVPixelBuffer> {
        let buffer = self
            .staging_buffer
            .take()
            .ok_or_else(|| anyhow!("readback was not staged"))?;
        let total = buffer.size() as usize;
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        let _ = self.wgpu_device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        rx.recv()
            .map_err(|_| anyhow!("readback map failed"))??;

        let data = slice.get_mapped_range();
        let mut bytes = vec![0u8; total];
        bytes.copy_from_slice(&data);
        drop(data);
        buffer.unmap();

        self.cpu_bytes = Some(bytes.into_boxed_slice());
        let owned = self.cpu_bytes.as_ref().unwrap().clone();
        let callback_ref: Box<Vec<u8>> = Box::new(owned.into_vec());
        let release_con = Box::into_raw(callback_ref) as *mut c_void;
        let pb = unsafe {
            CVPixelBuffer::new_with_bytes(
                self.format.ostype(),
                self.width as usize,
                self.height as usize,
                release_con as *mut c_void,
                self.readback_bytes_per_row as usize,
                free_bytes,
                release_con,
                None,
            )
        };
        pb.map_err(|status| anyhow!("CVPixelBufferCreateWithBytes failed: {status}"))
    }
}

impl SurfaceFormat {
    /// Bytes per pixel of this format.
    fn bytes_per_pixel(self) -> u32 {
        match self {
            SurfaceFormat::Bgra8Unorm => 4,
            SurfaceFormat::Rgba16Float => 8,
        }
    }
}

/// 64-byte alignment used by CoreVideo row buffers.
fn align_to_64(value: u32) -> u32 {
    (value + 63) & !63
}

/// Release callback for the readback path: frees the boxed byte vector.
extern "C" fn free_bytes(release_ref_con: *mut c_void, _base_address: *const *const c_void) {
    if !release_ref_con.is_null() {
        unsafe {
            drop(Box::from_raw(release_ref_con as *mut Vec<u8>));
        }
    }
}

/// Create an IOSurface-backed `CVPixelBuffer` (the IOSurface is created
/// internally by CoreVideo, which is what makes the buffer shareable with
/// Metal without any CPU copy).
fn create_iosurface_pixel_buffer(
    width: u32,
    height: u32,
    format: SurfaceFormat,
) -> Result<CVPixelBuffer> {
    let io_properties =
        CFDictionary::<CFString, core_foundation::base::CFType>::from_CFType_pairs(&[]);
    let attributes = CFDictionary::from_CFType_pairs(&[
        (
            unsafe { CFString::wrap_under_get_rule(kCVPixelBufferIOSurfacePropertiesKey) },
            io_properties.as_CFType(),
        ),
        (
            unsafe { CFString::wrap_under_get_rule(kCVPixelBufferMetalCompatibilityKey) },
            CFBoolean::true_value().as_CFType(),
        ),
    ]);
    CVPixelBuffer::new(
        format.ostype(),
        width as usize,
        height as usize,
        Some(&attributes),
    )
    .map_err(|status| anyhow!("CVPixelBufferCreate failed: {status}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_map_consistently() {
        for format in [SurfaceFormat::Bgra8Unorm, SurfaceFormat::Rgba16Float] {
            assert_eq!(format.metal(), format.wgpu().into_metal_pixel_format());
            assert!(format.ostype() != 0);
        }
    }

    #[test]
    fn row_alignment() {
        assert_eq!(align_to_64(64), 64);
        assert_eq!(align_to_64(66), 128);
        assert_eq!(align_to_64(0), 0);
    }

    trait IntoMetalPixelFormat {
        fn into_metal_pixel_format(self) -> MTLPixelFormat;
    }
    impl IntoMetalPixelFormat for wgpu::TextureFormat {
        fn into_metal_pixel_format(self) -> MTLPixelFormat {
            match self {
                wgpu::TextureFormat::Bgra8Unorm => MTLPixelFormat::BGRA8Unorm,
                wgpu::TextureFormat::Rgba16Float => MTLPixelFormat::RGBA16Float,
                _ => MTLPixelFormat::Invalid,
            }
        }
    }
}
