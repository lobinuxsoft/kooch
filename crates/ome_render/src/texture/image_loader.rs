//! PNG / JPEG image loader implementing [`AssetLoader<Image>`].
//!
//! Wraps the `image` crate (PNG + JPEG features only in PR-1). Decoded
//! input is normalized to RGBA8 — single-channel and RGB sources are
//! expanded so downstream code never branches on channel count.
//!
//! Format hint defaults to `Rgba8UnormSrgb` (color textures). Callers
//! that load *data* textures (normal maps, metal/rough, AO) build the
//! loader with [`ImageLoader::linear`] so the hint becomes `Rgba8Unorm`.

use ome_core::asset_loader::{AssetError, AssetLoader, AssetResult, LoadContext};

use super::asset::{Image, ImageFormat};

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

    fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<Image> {
        let dynamic = image::load_from_memory(bytes)
            .map_err(|e| AssetError::Loader(Box::new(ImageDecodeError(e))))?;
        // Normalize to RGBA8 — branch-free downstream upload.
        let rgba = dynamic.to_rgba8();
        let (width, height) = rgba.dimensions();
        Ok(Image::from_rgba8(
            rgba.into_raw(),
            width,
            height,
            self.format,
        ))
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
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::path::Path;

    /// Builds a 2×2 PNG with four distinct pixels for round-trip tests.
    fn build_test_png() -> Vec<u8> {
        let mut buf = Vec::new();
        let mut img = image::RgbaImage::new(2, 2);
        img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255])); // red
        img.put_pixel(1, 0, image::Rgba([0, 255, 0, 255])); // green
        img.put_pixel(0, 1, image::Rgba([0, 0, 255, 255])); // blue
        img.put_pixel(1, 1, image::Rgba([255, 255, 255, 255])); // white
        img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
            .expect("png encode should succeed");
        buf
    }

    #[test]
    fn extensions_includes_png_and_jpeg_variants() {
        let loader = ImageLoader::default();
        assert_eq!(loader.extensions(), &["png", "jpg", "jpeg"]);
    }

    #[test]
    fn srgb_constructor_uses_srgb_format() {
        assert_eq!(ImageLoader::srgb().format(), ImageFormat::Rgba8UnormSrgb);
    }

    #[test]
    fn linear_constructor_uses_unorm_format() {
        assert_eq!(ImageLoader::linear().format(), ImageFormat::Rgba8Unorm);
    }

    #[test]
    fn invalid_bytes_return_loader_error() {
        let loader = ImageLoader::srgb();
        let mut ctx = LoadContext {
            path: Path::new("bogus.png"),
        };
        let err = loader.load(b"not a real image", &mut ctx).unwrap_err();
        match err {
            AssetError::Loader(_) => {}
            other => panic!("expected Loader error, got {other:?}"),
        }
    }

    #[test]
    fn png_round_trip_preserves_pixels_and_dims() {
        let png = build_test_png();
        let loader = ImageLoader::srgb();
        let mut ctx = LoadContext {
            path: Path::new("test.png"),
        };
        let img = loader.load(&png, &mut ctx).expect("png decode");

        assert_eq!(img.width, 2);
        assert_eq!(img.height, 2);
        assert_eq!(img.format, ImageFormat::Rgba8UnormSrgb);

        // Pixels are row-major, RGBA, 4 bytes each.
        assert_eq!(img.data.len(), 16);
        assert_eq!(&img.data[0..4], &[255, 0, 0, 255]);
        assert_eq!(&img.data[4..8], &[0, 255, 0, 255]);
        assert_eq!(&img.data[8..12], &[0, 0, 255, 255]);
        assert_eq!(&img.data[12..16], &[255, 255, 255, 255]);
    }

    #[test]
    fn linear_loader_keeps_pixels_but_marks_format() {
        let png = build_test_png();
        let loader = ImageLoader::linear();
        let mut ctx = LoadContext {
            path: Path::new("normal.png"),
        };
        let img = loader.load(&png, &mut ctx).expect("png decode");
        assert_eq!(img.format, ImageFormat::Rgba8Unorm);
        assert_eq!(img.width, 2);
        assert_eq!(img.height, 2);
    }
}
