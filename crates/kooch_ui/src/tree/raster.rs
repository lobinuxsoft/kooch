//! Text into drawable quads: swash glyphs in the image atlas, or with `vello` one vector slot per
//! text node (#1880).

use std::hash::{Hash, Hasher};

use glam::Vec2;

use super::{GlyphQuad, UiNode, UiTree};
use crate::atlas::{GlyphAtlas, GlyphKey, subpixel};
use crate::text::UiFonts;

impl UiTree {
    /// Rebuilds the glyph quads when the layout, the scale, the atlas or a text's zoom changed.
    /// `scale` is physical pixels per canvas unit. Returns whether it ran.
    pub fn rasterize(&mut self, scale: f32, fonts: &mut UiFonts, atlas: &mut GlyphAtlas) -> bool {
        let zooms = self.text_zooms();
        if self.rastered == Some((scale, atlas.generation(), zooms)) {
            return false;
        }
        let before = std::mem::take(&mut self.held_texts);
        // Twice at most: a full atlas clears itself mid-pass, and what was built before points at
        // space it gave away.
        for _ in 0..2 {
            let generation = atlas.generation();
            self.glyphs.clear();
            self.held_texts = (generation, Vec::new());
            for index in 0..self.text.len() {
                if self.culled[index] || self.clipped(UiNode(index as u32)) {
                    continue;
                }
                let zoom = self.zoom_of(UiNode(index as u32));
                #[cfg(feature = "vello")]
                let path = self.text_path(UiNode(index as u32), scale);
                let Some(text) = self.text[index].as_mut() else {
                    continue;
                };
                let origin = text.origin;
                let layout = text.shaped(fonts);
                #[cfg(feature = "vello")]
                {
                    let place = super::vector_text::Place {
                        origin,
                        scale,
                        zoom,
                        node: index as u32,
                        view: self.view,
                    };
                    let (held, out) = (&mut self.held_texts.1, &mut self.glyphs);
                    match &path {
                        Some(path) => {
                            super::vector_text::push_bent(layout, place, path, atlas, out, held)
                        }
                        None => super::vector_text::push_text(layout, place, atlas, out, held),
                    }
                }
                #[cfg(not(feature = "vello"))]
                {
                    let _ = zoom;
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
            }
            if atlas.generation() == generation {
                // Given back after the new ones are held, so a text that did not change keeps its
                // slot; ones from before a clear are already gone.
                if before.0 == generation {
                    before.1.into_iter().for_each(|key| atlas.release_text(key));
                }
                self.rastered = Some((scale, generation, zooms));
                return true;
            }
        }
        true
    }

    /// A fingerprint of every text node's zoom, curve and the view: a `scale` animation redraws
    /// the text at its size, a reshaped curve resets its glyphs, a pan redraws the part shown.
    /// Always 0 without `vello`, whose glyphs stay as laid.
    fn text_zooms(&self) -> u64 {
        if !cfg!(feature = "vello") {
            return 0;
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.view
            .map(|view| view.map(f32::to_bits))
            .hash(&mut hasher);
        for (index, text) in self.text.iter().enumerate() {
            if text.is_some() {
                let node = UiNode(index as u32);
                self.zoom_of(node).to_bits().hash(&mut hasher);
                let line = self.bent_by(node).and_then(|curved| self.line(curved));
                for point in line.iter().flatten() {
                    point.to_array().map(f32::to_bits).hash(&mut hasher);
                }
            }
        }
        hasher.finish()
    }
}

#[cfg_attr(feature = "vello", allow(dead_code))]
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
                    vector: false,
                    bent: false,
                });
            }
        }
    }
}
