//! `Image` — CPU-side texture asset.
//!
//! Holds decoded pixel data + dimensions + format hint. The
//! [`ImageLoader`](super::ImageLoader) produces this; the
//! [`Image::upload`] method turns it into a [`GpuTexture`](super::GpuTexture)
//! when the renderer needs it.
//!
//! Splitting CPU `Image` from GPU [`GpuTexture`](super::GpuTexture) lets
//! the same asset be inspected by tools, baked offline, or re-uploaded
//! after edits without touching `wgpu` from the loader.

/// Pixel format hint accompanying the raw bytes. Mirrors the subset of
/// `wgpu::TextureFormat` we actually decode in PR-1. Compressed formats
/// (BC7, ASTC, ETC2) and HDR (`Rgba16Float`, `Rgba32Float`) arrive with
/// the KTX2 loader follow-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    /// 8-bit per channel sRGB (color textures: albedo, emissive).
    Rgba8UnormSrgb,
    /// 8-bit per channel linear (data textures: normal, metal/rough, AO).
    Rgba8Unorm,
}

impl ImageFormat {
    /// Maps to the matching `wgpu::TextureFormat` for upload.
    pub fn wgpu(self) -> wgpu::TextureFormat {
        match self {
            ImageFormat::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8UnormSrgb,
            ImageFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8Unorm,
        }
    }

    /// Bytes per pixel — used when computing row strides for upload.
    pub fn bytes_per_pixel(self) -> u32 {
        match self {
            ImageFormat::Rgba8UnormSrgb | ImageFormat::Rgba8Unorm => 4,
        }
    }
}

/// CPU-side image data: tightly packed RGBA8 pixel array + dimensions
/// + format hint.
///
/// Always RGBA8 internally — [`super::ImageLoader`] expands greyscale,
/// RGB, and RGBA inputs into RGBA8 so downstream code never branches on
/// channel count. This matches what `wgpu` accepts as the default texture
/// format and keeps the upload path branch-free.
#[derive(Debug, Clone)]
pub struct Image {
    /// Raw bytes, row-major, no padding. Length = `width * height *
    /// format.bytes_per_pixel()`.
    pub data: Vec<u8>,
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// Format hint used at upload time.
    pub format: ImageFormat,
}

impl Image {
    /// Builds an image from raw RGBA8 bytes. Caller asserts
    /// `data.len() == width * height * 4`.
    pub fn from_rgba8(data: Vec<u8>, width: u32, height: u32, format: ImageFormat) -> Self {
        debug_assert_eq!(
            data.len(),
            (width as usize) * (height as usize) * format.bytes_per_pixel() as usize,
            "Image data length must match width * height * bytes_per_pixel",
        );
        Self {
            data,
            width,
            height,
            format,
        }
    }

    /// 1×1 image of `[r, g, b, a]`. Useful as a default / placeholder
    /// (white for albedo, [128, 128, 255, 255] for normal, etc.) so
    /// shaders never branch on missing-texture path.
    pub fn solid_color(rgba: [u8; 4], format: ImageFormat) -> Self {
        Self {
            data: rgba.to_vec(),
            width: 1,
            height: 1,
            format,
        }
    }

    /// Total byte count of the pixel buffer.
    pub fn byte_count(&self) -> usize {
        self.data.len()
    }
}

#[cfg(test)]
mod tests;
