//! A text node as one vector-atlas slot (#1880): parley's glyph runs, drawn by Vello from the font's
//! own outlines at the pixels the node covers after its transform, so a zoom stays sharp. The bytes
//! parley already holds are the font: nothing is derived from it.

use std::hash::{Hash, Hasher};

use glam::Vec2;
use kooch_ecs::{UiFontSource, UiGlyphRun, UiVectorText};

use super::GlyphQuad;
use super::text_path::TextPath;
use crate::atlas::GlyphAtlas;

/// The longest side a text slot is drawn at whole; past it, only what the view shows is drawn, or
/// with a transform or no view, it is stretched.
const LARGEST: f32 = 2048.0;

/// Where a text node is drawn: its first line's top-left in canvas units, physical pixels per
/// canvas unit, the transform's magnification and the node.
pub(super) struct Place {
    pub origin: Vec2,
    pub scale: f32,
    pub zoom: f32,
    pub node: u32,
    /// The physical pixels on screen, which crop a text too large to draw whole.
    pub view: Option<[f32; 4]>,
}

/// Holds the slot of `layout` drawn at `place`, and pushes the quad that shows it.
pub(super) fn push_text(
    layout: &parley::Layout<()>,
    place: Place,
    atlas: &mut GlyphAtlas,
    out: &mut Vec<GlyphQuad>,
    held: &mut Vec<u64>,
) {
    let laid = Vec2::new(layout.full_width().max(layout.width()), layout.height()) * place.scale;
    let cropped = place
        .view
        .filter(|_| (place.zoom - 1.0).abs() < 1e-3 && laid.max_element() > LARGEST);
    let zoom = match cropped {
        Some(_) => place.zoom,
        None => place.zoom.min(LARGEST / laid.max_element().max(1.0)),
    };
    let factor = place.scale * zoom;
    // Whole pixels for the quad, the fraction moved into the glyphs: at zoom 1 they land as laid.
    let at = place.origin * place.scale;
    let snapped = at.floor();
    let fraction = (at - snapped) * zoom;
    let (mut runs, largest) = runs_of(layout, factor, |pen, _, _| (pen.into(), None));
    if runs.is_empty() {
        return;
    }
    // Room for what hangs past the line box: an italic's overhang, a descender.
    let pad = (largest * 0.25).ceil() + 1.0;
    let shift = fraction + Vec2::splat(pad);
    for (_, run) in &mut runs {
        for (_, at) in &mut run.glyphs {
            *at = [at[0] + shift.x, at[1] + shift.y];
        }
    }
    let mut size = (laid * zoom + fraction + Vec2::splat(2.0 * pad)).ceil();
    let mut min = snapped - Vec2::splat(pad) / zoom;
    if let Some(view) = cropped {
        let whole = [min.x, min.y, min.x + size.x, min.y + size.y];
        if let Some(crop) = crate::crops::crop_in(whole, view, 1.0) {
            if crop.size.contains(&0) {
                return;
            }
            let from = Vec2::new(crop.from[0] as f32, crop.from[1] as f32);
            for (_, run) in &mut runs {
                for (_, at) in &mut run.glyphs {
                    *at = [at[0] - from.x, at[1] - from.y];
                }
            }
            min += from;
            size = Vec2::new(crop.size[0] as f32, crop.size[1] as f32);
        }
    }
    let slot = Slot {
        min,
        size,
        zoom,
        bent: false,
    };
    push_slot(runs, slot, place.node, atlas, out, held);
}

/// `layout` set along `path`, each glyph turned to the line where its middle lands.
pub(super) fn push_bent(
    layout: &parley::Layout<()>,
    place: Place,
    path: &TextPath,
    atlas: &mut GlyphAtlas,
    out: &mut Vec<GlyphQuad>,
    held: &mut Vec<u64>,
) {
    let origin = place.origin * place.scale;
    let (mut low, mut high) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    let (mut runs, _) = runs_of(layout, place.scale, |pen, advance, size| {
        let half = advance * 0.5;
        let (middle, unit) = path.at(origin + pen + Vec2::new(half, 0.0));
        // Ascender, descender and advance all fit in a glyph's em about its middle.
        let reach = Vec2::splat(half.hypot(size) + 1.0);
        low = low.min(middle - reach);
        high = high.max(middle + reach);
        ((middle - unit * half).into(), Some(unit.into()))
    });
    if runs.is_empty() {
        return;
    }
    let min = low.floor();
    let laid = high - min;
    let zoom = place.zoom.min(LARGEST / laid.max_element().max(1.0));
    for (_, run) in &mut runs {
        run.size *= zoom;
        for (_, at) in &mut run.glyphs {
            *at = ((Vec2::from(*at) - min) * zoom).into();
        }
    }
    let size = (laid * zoom).ceil();
    let slot = Slot {
        min,
        size,
        zoom,
        bent: true,
    };
    push_slot(runs, slot, place.node, atlas, out, held);
}

/// Where a text slot lands: `min` in physical pixels, `size` in texels drawn `zoom` times larger.
struct Slot {
    min: Vec2,
    size: Vec2,
    zoom: f32,
    bent: bool,
}

/// Each run of `layout`, its glyphs at what `place` makes of their pen position, advance and
/// pixels per em, all `factor` times the layout's; and the largest pixels per em.
fn runs_of(
    layout: &parley::Layout<()>,
    factor: f32,
    mut place: impl FnMut(Vec2, f32, f32) -> ([f32; 2], Option<[f32; 2]>),
) -> (Vec<(parley::FontData, UiGlyphRun)>, f32) {
    let mut runs = Vec::new();
    let mut largest = 0.0_f32;
    for line in layout.lines() {
        for item in line.items() {
            let parley::PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let shaped = run.run();
            let font = shaped.font();
            let size = shaped.font_size() * factor;
            largest = largest.max(size);
            let (mut glyphs, mut turns) = (Vec::new(), Vec::new());
            for glyph in run.positioned_glyphs() {
                let pen = Vec2::new(glyph.x, glyph.y) * factor;
                let (at, turn) = place(pen, glyph.advance * factor, size);
                glyphs.push((glyph.id, at));
                turns.extend(turn);
            }
            let coords = shaped.normalized_coords().iter().map(|c| c.to_bits());
            runs.push((
                font.clone(),
                UiGlyphRun {
                    font: font.data.id(),
                    index: font.index,
                    size,
                    coords: coords.collect(),
                    glyphs,
                    turns,
                },
            ));
        }
    }
    (runs, largest)
}

/// Holds the slot of `runs` and pushes the quad that shows it.
fn push_slot(
    runs: Vec<(parley::FontData, UiGlyphRun)>,
    slot: Slot,
    node: u32,
    atlas: &mut GlyphAtlas,
    out: &mut Vec<GlyphQuad>,
    held: &mut Vec<u64>,
) {
    let (w, h) = (slot.size.x as u32, slot.size.y as u32);
    let key = key_of(&runs, [w, h]);
    let mut fonts: Vec<UiFontSource> = Vec::new();
    for (font, _) in &runs {
        let id = font.data.id();
        if !atlas.has_font(id) && fonts.iter().all(|known| known.id != id) {
            fonts.push(UiFontSource {
                id,
                data: font.data.data().into(),
            });
        }
    }
    let text = || UiVectorText {
        key,
        size: [w as f32, h as f32],
        runs: runs.iter().map(|(_, run)| run.clone()).collect(),
    };
    let Some(image) = atlas.hold_text(key, (w, h), text, fonts) else {
        return;
    };
    held.push(key);
    let [x0, y0, x1, y1] = image.rect;
    out.push(GlyphQuad {
        min: slot.min.into(),
        size: (slot.size / slot.zoom).into(),
        uv_min: [x0 as u16, y0 as u16],
        uv_size: [(x1 - x0) as u16, (y1 - y0) as u16],
        node,
        vector: true,
        bent: slot.bent,
    });
}

/// What makes two text slots the same drawing.
fn key_of(runs: &[(parley::FontData, UiGlyphRun)], size: [u32; 2]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    size.hash(&mut hasher);
    for (_, run) in runs {
        (run.font, run.index, run.size.to_bits(), &run.coords).hash(&mut hasher);
        for (glyph, at) in &run.glyphs {
            (glyph, at[0].to_bits(), at[1].to_bits()).hash(&mut hasher);
        }
        for turn in &run.turns {
            turn.map(f32::to_bits).hash(&mut hasher);
        }
    }
    hasher.finish()
}
