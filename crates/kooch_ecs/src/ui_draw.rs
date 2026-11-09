//! What the UI hands the renderer each frame (#96): quads in physical pixels and the glyph atlas's
//! changed region. Plain data, so the same list can cross to the editor in remote Play.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use kooch_core::Guid;

/// A quad samples the atlas's coverage instead of drawing a rounded box.
pub const QUAD_GLYPH: u32 = 1;
/// A quad samples an image from the atlas, filtered, tinted by `color` (#1708).
pub const QUAD_IMAGE: u32 = 2;
/// With [`QUAD_IMAGE`]: the texels are in the vector atlas Vello draws, not the glyph atlas (#1865).
/// 4 is `QUAD_CURVED`, which `ui.wgsl` sets itself.
pub const QUAD_VECTOR: u32 = 8;
/// In [`UiMask::luminance`]: the mask image's texels are in the vector atlas.
pub const MASK_VECTOR: u32 = 2;
/// Evenly spaced points in a [`UiCurve`]; `ui.wgsl` holds the same number.
pub const CURVE_POINTS: usize = 128;

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
    /// Index in [`UiDrawList::curves`] plus one; 0 is straight.
    pub curve: u32,
    /// Index in [`UiDrawList::mirrors`] plus one; 0 is as laid out.
    pub mirror: u32,
}

/// Where the flips and transforms around a quad move it, in physical pixels: `before` its curve
/// bends it, then `after`. Each is a 3x3 matrix by columns, padded as WGSL lays out a `mat3x3`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct UiMirror {
    pub before: [[f32; 4]; 3],
    pub after: [[f32; 4]; 3],
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
    /// Bit 1: the image cuts by how bright it is too, not by its alpha alone. [`MASK_VECTOR`]: its
    /// texels are in the vector atlas.
    pub luminance: u32,
    /// Where a fill starts, as an index in `-kooch-fill-from`'s words; 0 cuts no fill (#1829).
    pub fill_from: u32,
    /// How much of it shows, 0 to 1.
    pub fill: f32,
    /// WGSL rounds the struct to 8 bytes.
    pub pad: f32,
}

/// A line the quads inside a node bend along (#1827), in physical pixels. Resampled by length, so
/// the GPU does no curve maths and a longer curve changes nothing there.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct UiCurve {
    pub points: [[f32; 2]; CURVE_POINTS],
    /// The straight span that maps onto the line: `left` to `left + width`.
    pub left: f32,
    pub width: f32,
    /// The straight height that lands on the line itself.
    pub middle: f32,
    /// 1 when the line ends where it starts, and the bend carries through the seam.
    pub closed: u32,
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

/// An SVG Vello draws into the vector atlas (#1865): `key` names it across frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiVector {
    pub key: u64,
    /// Its texels, `[x0, y0, x1, y1]`.
    pub rect: [u32; 4],
}

/// The file behind a [`UiVector`], sent once when it is first packed or changes.
#[derive(Debug, Clone, PartialEq)]
pub struct UiVectorSource {
    pub key: u64,
    pub svg: Arc<[u8]>,
}

/// A text node's glyphs behind a [`UiVector`] (#1880), drawn white in a slot of `size` texels.
#[derive(Debug, Clone, PartialEq)]
pub struct UiVectorText {
    pub key: u64,
    pub size: [f32; 2],
    pub runs: Vec<UiGlyphRun>,
}

/// Glyphs of one font at one size, positioned in the slot's texels.
#[derive(Debug, Clone, PartialEq)]
pub struct UiGlyphRun {
    /// A [`UiFontSource::id`].
    pub font: u64,
    /// The face in a collection.
    pub index: u32,
    /// Pixels per em.
    pub size: f32,
    /// Variable-font coordinates, normalized 2.14.
    pub coords: Vec<i16>,
    /// Glyph id and its pen position.
    pub glyphs: Vec<(u32, [f32; 2])>,
}

/// A font file, sent once before the first text that uses it.
#[derive(Debug, Clone, PartialEq)]
pub struct UiFontSource {
    pub id: u64,
    pub data: Arc<[u8]>,
}

/// This frame's UI, back to front.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UiDrawList {
    pub quads: Vec<UiQuad>,
    pub masks: Vec<UiMask>,
    pub curves: Vec<UiCurve>,
    pub mirrors: Vec<UiMirror>,
    pub materials: Vec<UiMaterial>,
    pub params: Vec<UiParam>,
    pub shaders: Vec<UiShader>,
    /// Side of the square RGBA8 atlas: glyph coverage and images share it.
    pub atlas_side: u32,
    /// What to write into the atlas before drawing; `None` when nothing changed.
    pub atlas_upload: Option<AtlasUpload>,
    /// Side of the square vector atlas; 0 while no SVG is packed there.
    pub vector_side: u32,
    /// Every SVG resident in the vector atlas. The renderer redraws it when this changes.
    pub vectors: Vec<UiVector>,
    /// Files not handed over yet; the renderer takes them, as it takes `atlas_upload`.
    pub vector_sources: Vec<UiVectorSource>,
    /// Text not handed over yet, as `vector_sources`.
    pub vector_texts: Vec<UiVectorText>,
    pub vector_fonts: Vec<UiFontSource>,
}
