//! What the UI hands the renderer each frame (#96): quads in physical pixels and the glyph atlas's
//! changed region. Plain data, so the same list can cross to the editor in remote Play.

use bytemuck::{Pod, Zeroable};

/// A quad samples the atlas's coverage instead of drawing a rounded box.
pub const QUAD_GLYPH: u32 = 1;
/// A quad samples an image from the atlas, filtered, tinted by `color` (#1708).
pub const QUAD_IMAGE: u32 = 2;

/// One UI rectangle: a panel, or one glyph of a text.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Pod, Zeroable)]
pub struct UiQuad {
    pub min: [f32; 2],
    pub size: [f32; 2],
    /// `[x0, y0, x1, y1]`: what is outside is not drawn.
    pub clip: [f32; 4],
    /// sRGB, straight alpha: the fill, or the text's colour. Blended in display space, like the
    /// non-sRGB surface it lands on.
    pub color: [f32; 4],
    pub border_color: [f32; 4],
    /// Corner radius and border width, in physical pixels.
    pub radius: f32,
    pub border: f32,
    /// The glyph's or image's atlas texels, `min` and `size`.
    pub uv_min: [u32; 2],
    pub uv_size: [u32; 2],
    pub material: u32,
    pub flags: u32,
}

/// A rectangle of the atlas that changed, and its RGBA8 texels, row by row.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AtlasUpload {
    pub rect: [u32; 4],
    pub texels: Vec<u8>,
}

/// This frame's UI, back to front.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UiDrawList {
    pub quads: Vec<UiQuad>,
    /// Side of the square RGBA8 atlas: glyph coverage and images share it.
    pub atlas_side: u32,
    /// What to write into the atlas before drawing; `None` when nothing changed.
    pub atlas_upload: Option<AtlasUpload>,
}
