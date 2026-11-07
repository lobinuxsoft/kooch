//! SVG images (#1700): drawn to RGBA8 on the CPU at the size whoever shows them needs, so they
//! stay sharp at any scale and reach the GPU like any other image.

use resvg::{tiny_skia, usvg};

/// The longest side a raster may have: past it, a huge node would eat the atlas.
pub const MAX_SIDE: u32 = 4096;

/// Straight-alpha RGBA8, row by row, as a decoded PNG is.
#[derive(Debug, Clone)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// The size the file declares, in its own units.
    pub own: (f32, f32),
}

/// The size the file declares, in its own units.
pub fn intrinsic_size(bytes: &[u8]) -> Result<(f32, f32), String> {
    let size = parse(bytes)?.size();
    Ok((size.width(), size.height()))
}

/// The image stretched to `size` pixels, or at its own size; each side kept to 1..=`MAX_SIDE`.
pub fn rasterize(bytes: &[u8], size: Option<(u32, u32)>) -> Result<Raster, String> {
    let tree = parse(bytes)?;
    let own = tree.size();
    let (width, height) = size.unwrap_or((own.width().ceil() as u32, own.height().ceil() as u32));
    let (width, height) = (width.clamp(1, MAX_SIDE), height.clamp(1, MAX_SIDE));
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| format!("no {width}x{height} pixmap"))?;
    let stretch =
        tiny_skia::Transform::from_scale(width as f32 / own.width(), height as f32 / own.height());
    resvg::render(&tree, stretch, &mut pixmap.as_mut());
    // tiny-skia keeps alpha premultiplied; the atlas and the texture upload expect it straight.
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let c = pixel.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    Ok(Raster {
        width,
        height,
        rgba,
        own: (own.width(), own.height()),
    })
}

/// Whether `path` names an SVG, which no other image decoder reads.
pub fn is_svg(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
}

fn parse(bytes: &[u8]) -> Result<usvg::Tree, String> {
    usvg::Tree::from_data(bytes, &usvg::Options::default()).map_err(|e| format!("SVG: {e}"))
}
