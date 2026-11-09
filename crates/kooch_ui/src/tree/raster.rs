//! Text into drawable quads: swash glyphs in the image atlas, or with `vello` one vector slot per
//! text node (#1880).

use std::hash::{Hash, Hasher};

use glam::Vec2;

use super::{GlyphQuad, UiNode, UiTree};
use crate::atlas::{GlyphAtlas, GlyphKey, subpixel};
use crate::text::UiFonts;

impl UiTree {
    /// Rebuilds the glyph quads when the layout, the scale or the atlas changed. `scale` is
    /// physical pixels per canvas unit. Returns whether it ran.
    pub fn rasterize(&mut self, scale: f32, fonts: &mut UiFonts, atlas: &mut GlyphAtlas) -> bool {
        if self.rastered == Some((scale, atlas.generation())) {
            return false;
        }
        // Twice at most: a full atlas clears itself mid-pass, and what was built before points at
        // space it gave away.
        for _ in 0..2 {
            let generation = atlas.generation();
            self.glyphs.clear();
            for index in 0..self.text.len() {
                if self.culled[index] || self.clipped(UiNode(index as u32)) {
                    continue;
                }
                let Some(text) = self.text[index].as_mut() else {
                    continue;
                };
                let origin = text.origin;
                let layout = text.shaped(fonts);
                push_glyphs(
                    layout,
                    origin,
                    scale,
                    index as u32,
                    &mut fonts.scale,
                    atlas,
                    &mut self.glyphs,
                );
            }
            if atlas.generation() == generation {
                self.rastered = Some((scale, generation));
                return true;
            }
        }
        true
    }
}

fn push_glyphs(
    layout: &parley::Layout<()>,
    origin: Vec2,
    scale: f32,
    node: u32,
    context: &mut swash::scale::ScaleContext,
    atlas: &mut GlyphAtlas,
    out: &mut Vec<GlyphQuad>,
) {
    for line in layout.lines() {
        for item in line.items() {
            let parley::PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let shaped = run.run();
            let font = shaped.font();
            let Some(font_ref) = swash::FontRef::from_index(font.data.data(), font.index as usize)
            else {
                continue;
            };
            let coords: Vec<i16> = shaped
                .normalized_coords()
                .iter()
                .map(|coord| coord.to_bits())
                .collect();
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            coords.hash(&mut hasher);
            let coords_hash = hasher.finish();
            let size = (shaped.font_size() * scale * 64.0).round() as u32;
            for glyph in run.positioned_glyphs() {
                let x = (origin.x + glyph.x) * scale;
                let y = ((origin.y + glyph.y) * scale).round();
                let (pixel, bin) = subpixel(x);
                let key = GlyphKey {
                    font: font.data.id(),
                    index: font.index,
                    glyph: glyph.id as u16,
                    size,
                    subpixel: bin,
                    coords: coords_hash,
                };
                let Some(slot) = atlas.glyph(context, font_ref, &coords, key) else {
                    continue;
                };
                out.push(GlyphQuad {
                    min: [pixel + slot.left as f32, y - slot.top as f32],
                    size: [slot.size[0] as f32, slot.size[1] as f32],
                    uv_min: slot.min,
                    uv_size: slot.size,
                    node,
                });
            }
        }
    }
}
