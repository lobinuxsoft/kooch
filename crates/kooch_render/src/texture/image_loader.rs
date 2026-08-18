//! PNG / JPEG image loader implementing [`AssetLoader<Image>`].
//!
//! Wraps the `image` crate (PNG + JPEG features only in PR-1). Decoded
//! input is normalized to RGBA8 — single-channel and RGB sources are
//! expanded so downstream code never branches on channel count.
//!
//! Format hint defaults to `Rgba8UnormSrgb` (color textures). Callers
//! that load *data* textures (normal maps, metal/rough, AO) build the
//! loader with [`ImageLoader::linear`] so the hint becomes `Rgba8Unorm`.

use kooch_core::asset_loader::{AssetError, AssetLoader, AssetResult, LoadContext};
use serde::{Deserialize, Serialize};

use super::asset::{Image, ImageFormat};

/// What a texture's `.meta` may say about how it is imported.
///
/// ```toml
/// guid = "..."
/// asset_type = "kooch_render::texture::asset::Image"
///
/// [import]
/// mipmaps = false
/// ```
///
/// 🔴 The default is ON, and it is the answer for almost every texture:
/// anything seen in perspective aliases without a chain, and the ones
/// that do not want one are the exceptions — a UI atlas sampled 1:1, a
/// lookup table whose neighbouring texels are unrelated values, a
/// gradient ramp read by index. Those say so; everything else says
/// nothing and gets the right thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ImageImport {
    /// Whether the upload builds the mip chain.
    pub mipmaps: bool,
}

impl Default for ImageImport {
    fn default() -> Self {
        Self { mipmaps: true }
    }
}

/// Configurable PNG/JPEG loader.
///
/// Two construction paths cover the common case:
/// - [`ImageLoader::srgb`] — color textures (albedo, emissive). Default.
/// - [`ImageLoader::linear`] — data textures (normal, ORM, AO).
#[derive(Debug, Clone, Copy)]
pub struct ImageLoader {
    format: ImageFormat,
}

impl ImageLoader {
    /// Loader emitting `Rgba8UnormSrgb` images (color textures).
    pub fn srgb() -> Self {
        Self {
            format: ImageFormat::Rgba8UnormSrgb,
        }
    }

    /// Loader emitting `Rgba8Unorm` images (data textures: normal,
    /// metal/roughness, AO, height, etc.).
    pub fn linear() -> Self {
        Self {
            format: ImageFormat::Rgba8Unorm,
        }
    }

    /// Format hint applied to every image this loader produces.
    pub fn format(&self) -> ImageFormat {
        self.format
    }
}

impl Default for ImageLoader {
    fn default() -> Self {
        Self::srgb()
    }
}

impl AssetLoader<Image> for ImageLoader {
    fn extensions(&self) -> &[&'static str] {
        &["png", "jpg", "jpeg"]
    }

    fn load(&self, bytes: &[u8], ctx: &mut LoadContext<'_>) -> AssetResult<Image> {
        let dynamic = image::load_from_memory(bytes)
            .map_err(|e| AssetError::Loader(Box::new(ImageDecodeError(e))))?;
        // Normalize to RGBA8 — branch-free downstream upload.
        let rgba = dynamic.to_rgba8();
        let (width, height) = rgba.dimensions();
        let import: ImageImport = ctx.import();
        let image = Image::from_rgba8(rgba.into_raw(), width, height, self.format);
        Ok(if import.mipmaps {
            image
        } else {
            image.without_mipmaps()
        })
    }
}

/// Wrapper so `image::ImageError` lives behind our `AssetError::Loader`
/// boundary without leaking the `image` crate into public types.
#[derive(Debug)]
struct ImageDecodeError(image::ImageError);

impl std::fmt::Display for ImageDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "image decode failed: {}", self.0)
    }
}

impl std::error::Error for ImageDecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

#[cfg(test)]
mod tests;
