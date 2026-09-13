//! The window icon every Kóoch window carries; engine-side so games get it too.
//! The single-hue `kooch_myth.svg`, not `kooch_debug.svg`: the debug mark turns to mush below ~48
//! px, and a title bar is 16–32.

use winit::window::Icon;

/// 64×64 RGBA from `docs/brand/kooch_myth.svg` — the task switcher uses the full size, and
/// downscaling beats upscaling.
const ICON_PNG: &[u8] = include_bytes!("../icon/kooch-64.png");

/// Decodes the embedded icon; `None` rather than a panic, since a missing icon is cosmetic.
pub fn window_icon() -> Option<Icon> {
    let decoded = match image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png) {
        Ok(image) => image.to_rgba8(),
        Err(err) => {
            tracing::warn!("window icon failed to decode ({err}); running without one");
            return None;
        }
    };
    let (width, height) = decoded.dimensions();
    match Icon::from_rgba(decoded.into_raw(), width, height) {
        Ok(icon) => Some(icon),
        Err(err) => {
            tracing::warn!("window icon rejected by winit: {err}");
            None
        }
    }
}

#[cfg(test)]
mod tests;
