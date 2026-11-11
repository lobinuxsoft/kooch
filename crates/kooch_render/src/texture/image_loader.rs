//! PNG / JPEG / SVG image loader implementing [`AssetLoader<Image>`].

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
    /// Texels each edge keeps unstretched when the UI draws it, top-right-bottom-left (#1792).
    /// Every use of the image slices by it; `-kooch-slice` overrides it.
    #[serde(skip_serializing_if = "is_unsliced")]
    pub slice: [u16; 4],
}

impl Default for ImageImport {
    fn default() -> Self {
        Self {
            mipmaps: true,
            slice: [0; 4],
        }
    }
}

fn is_unsliced(slice: &[u16; 4]) -> bool {
    *slice == [0; 4]
}

/// Configurable PNG/JPEG loader.
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
        &["png", "jpg", "jpeg", "svg", "gif"]
    }

    fn load(&self, bytes: &[u8], ctx: &mut LoadContext<'_>) -> AssetResult<Image> {
        // An SVG as a texture is drawn at the size it declares; the UI draws it at its own (#1700).
        let (rgba, width, height) = if kooch_core::svg::is_svg(ctx.path) {
            let raster = kooch_core::svg::rasterize(bytes, None)
                .map_err(|e| AssetError::Loader(e.into()))?;
            (raster.rgba, raster.width, raster.height)
        } else {
            let dynamic = image::load_from_memory(bytes)
                .map_err(|e| AssetError::Loader(Box::new(ImageDecodeError(e))))?;
            // Normalize to RGBA8 — branch-free downstream upload.
            let rgba = dynamic.to_rgba8();
            let (width, height) = rgba.dimensions();
            (rgba.into_raw(), width, height)
        };
        let import: ImageImport = ctx.import();
        let mut image = Image::from_rgba8(rgba, width, height, self.format);
        // A GIF's first frame is the texture; the pool plays the rest over it.
        if kooch_core::gif::is_gif(ctx.path) {
            image.animation = kooch_core::gif::decode(bytes)
                .map_err(|e| AssetError::Loader(e.into()))?
                .map(std::sync::Arc::new);
        }
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
