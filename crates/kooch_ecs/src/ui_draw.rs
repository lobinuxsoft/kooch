//! What the UI hands the renderer each frame (#96): quads in physical pixels and the glyph atlas's
//! changed region. Plain data, so the same list can cross to the editor in remote Play.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use kooch_core::Guid;

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
    /// Its index in [`UiDrawList::materials`]; unread on a quad none claims.
    pub material: u32,
    pub flags: u32,
    /// Index in [`UiDrawList::masks`] plus one; 0 is unmasked.
    pub mask: u32,
}

/// A shape what is inside a node gets cut to (#1790), in physical pixels.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Pod, Zeroable)]
pub struct UiMask {
    pub min: [f32; 2],
    pub size: [f32; 2],
    pub radius: f32,
    /// The mask around this one, as in [`UiQuad::mask`]: masks nest by walking up.
    pub parent: u32,
    /// The atlas texels whose alpha cuts too; a zero size is the shape alone.
    pub uv_min: [u32; 2],
    pub uv_size: [u32; 2],
}

/// A rectangle of the atlas that changed, and its RGBA8 texels, row by row.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AtlasUpload {
    pub rect: [u32; 4],
    pub texels: Vec<u8>,
}

/// A `kind: ui` shader as written: the draw list carries the text, so whoever draws needs no project.
#[derive(Debug, Clone, PartialEq)]
pub struct UiShader {
    pub guid: Guid,
    pub source: Arc<str>,
}

/// A shader parameter a node sets; the renderer matches `name` against what the shader declares.
#[derive(Debug, Clone, PartialEq)]
pub struct UiParam {
    pub name: Arc<str>,
    pub value: [f32; 4],
}

/// A quad drawn by a shader of its own.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct UiMaterial {
    /// Index in [`UiDrawList::quads`]. Materials go in quad order.
    pub quad: u32,
    /// Index in [`UiDrawList::shaders`].
    pub shader: u32,
    /// `first` and `count` in [`UiDrawList::params`].
    pub params: [u32; 2],
}

/// This frame's UI, back to front.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UiDrawList {
    pub quads: Vec<UiQuad>,
    pub masks: Vec<UiMask>,
    pub materials: Vec<UiMaterial>,
    pub params: Vec<UiParam>,
    pub shaders: Vec<UiShader>,
    /// Side of the square RGBA8 atlas: glyph coverage and images share it.
    pub atlas_side: u32,
    /// What to write into the atlas before drawing; `None` when nothing changed.
    pub atlas_upload: Option<AtlasUpload>,
}
