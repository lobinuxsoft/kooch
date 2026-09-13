use super::*;

/// The size `kooch-64.png` is rendered at.
const ICON_SIZE: u32 = 64;

/// A mark on transparency — corners empty, middle not — so a garbage decode or a full-tile logo
/// fails, not just a length check.
#[test]
fn the_embedded_icon_is_a_mark_on_transparency() {
    let image = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png)
        .expect("the icon shipped in this crate must decode")
        .to_rgba8();
    assert_eq!(image.dimensions(), (ICON_SIZE, ICON_SIZE));

    let alpha = |x: u32, y: u32| image.get_pixel(x, y).0[3];
    assert_eq!(alpha(0, 0), 0, "top-left corner should be transparent");
    assert_eq!(
        alpha(ICON_SIZE - 1, 0),
        0,
        "top-right corner should be transparent",
    );
    assert!(
        alpha(ICON_SIZE / 2, ICON_SIZE / 2) > 200,
        "the centre of the drop should be opaque",
    );
}

#[test]
fn winit_accepts_it() {
    assert!(window_icon().is_some(), "winit rejected the icon");
}
