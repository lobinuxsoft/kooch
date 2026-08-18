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
    let mut ctx = LoadContext::new(Path::new("bogus.png"));
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
    let mut ctx = LoadContext::new(Path::new("test.png"));
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
    let mut ctx = LoadContext::new(Path::new("normal.png"));
    let img = loader.load(&png, &mut ctx).expect("png decode");
    assert_eq!(img.format, ImageFormat::Rgba8Unorm);
    assert_eq!(img.width, 2);
    assert_eq!(img.height, 2);
}

/// A texture with nothing to say gets the chain.
///
/// 🔴 This is the direction that matters. Every texture in every project
/// that existed before the setting did has a `.meta` with no `[import]`
/// table, and the whole point is that those come back with mip chains
/// rather than staying broken until someone edits 78 files.
#[test]
fn silence_means_mipmaps() {
    let png = build_test_png();
    let mut ctx = LoadContext::new(Path::new("test.png"));
    let img = ImageLoader::srgb()
        .load(&png, &mut ctx)
        .expect("png decode");
    assert!(img.mipmaps);
    assert!(ImageImport::default().mipmaps);
}

/// And a texture that refuses one is obeyed.
#[test]
fn the_meta_can_refuse_the_chain() {
    let png = build_test_png();
    let table: kooch_core::toml::Table = "mipmaps = false".parse().expect("toml");
    let mut ctx = LoadContext::with_import(Path::new("test.png"), Some(&table));
    let img = ImageLoader::srgb()
        .load(&png, &mut ctx)
        .expect("png decode");
    assert!(
        !img.mipmaps,
        "the [import] table said no and the loader built the chain anyway",
    );
}

/// A malformed table leaves the texture on screen.
///
/// A settings file must never be the reason an asset fails to load: a
/// `.meta` written by a newer engine, or hand-edited with a typo, has to
/// degrade to the defaults. The complaint belongs in the log.
#[test]
fn a_malformed_import_falls_back() {
    let png = build_test_png();
    let table: kooch_core::toml::Table = "mipmaps = \"yes please\"".parse().expect("toml");
    let mut ctx = LoadContext::with_import(Path::new("test.png"), Some(&table));
    let img = ImageLoader::srgb()
        .load(&png, &mut ctx)
        .expect("png decode");
    assert!(img.mipmaps, "the fallback is the default, not nothing");
}
