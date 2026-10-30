//! Rasterized glyphs, packed into one `R8` atlas the draw samples (#96 phase 4 uploads it).

use std::collections::HashMap;

use etagere::{AtlasAllocator, size2};
use swash::FontRef;
use swash::scale::{Render, Source, StrikeWith};
use swash::zeno::{Format, Vector};

/// Fractions of a pixel a glyph's x is snapped to: four is where more stops being visible.
const SUBPIXEL: f32 = 4.0;

/// What makes two rasterized glyphs the same bitmap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub font: u64,
    pub index: u32,
    pub glyph: u16,
    /// Physical pixels per em, in 1/64ths.
    pub size: u32,
    pub subpixel: u8,
    /// Hash of the variable-font coordinates: Inter's weight is one.
    pub coords: u64,
}

/// Where a glyph sits in the atlas, and how it hangs off its pen position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtlasGlyph {
    pub min: [u16; 2],
    pub size: [u16; 2],
    pub left: i32,
    pub top: i32,
}

/// The atlas: pixels, what is where, and what changed since the last upload.
pub struct GlyphAtlas {
    allocator: AtlasAllocator,
    side: u32,
    pixels: Vec<u8>,
    /// `None`: a glyph with no ink (a space). Cached too, so it is not rasterized every frame.
    glyphs: HashMap<GlyphKey, Option<AtlasGlyph>>,
    /// Bounds of what was written since [`take_dirty`](Self::take_dirty), `[x0, y0, x1, y1]`.
    dirty: Option<[u32; 4]>,
    /// Bumped when a full atlas is cleared: every quad built before points at freed space.
    generation: u32,
}

impl Default for GlyphAtlas {
    fn default() -> Self {
        Self::new(1024)
    }
}

impl GlyphAtlas {
    pub fn new(side: u32) -> Self {
        Self {
            allocator: AtlasAllocator::new(size2(side as i32, side as i32)),
            side,
            pixels: vec![0; (side * side) as usize],
            glyphs: HashMap::new(),
            dirty: None,
            generation: 0,
        }
    }

    pub fn side(&self) -> u32 {
        self.side
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }

    pub fn len(&self) -> usize {
        self.glyphs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    /// The region written since the last call, for the upload.
    pub fn take_dirty(&mut self) -> Option<[u32; 4]> {
        self.dirty.take()
    }

    /// The glyph's slot, rasterizing it on first use. A full atlas is cleared and starts over: the
    /// glyphs a frame needs fit, and the ones it stopped needing are what made it full.
    pub(crate) fn glyph(
        &mut self,
        scale: &mut swash::scale::ScaleContext,
        font: FontRef<'_>,
        coords: &[i16],
        key: GlyphKey,
    ) -> Option<AtlasGlyph> {
        if let Some(cached) = self.glyphs.get(&key) {
            return *cached;
        }
        let mut scaler = scale
            .builder(font)
            .size(key.size as f32 / 64.0)
            .hint(true)
            .normalized_coords(coords)
            .build();
        let image = Render::new(&[
            Source::ColorOutline(0),
            Source::ColorBitmap(StrikeWith::BestFit),
            Source::Outline,
        ])
        .format(Format::Alpha)
        .offset(Vector::new(key.subpixel as f32 / SUBPIXEL, 0.0))
        .render(&mut scaler, key.glyph)?;
        let (w, h) = (image.placement.width, image.placement.height);
        if w == 0 || h == 0 {
            self.glyphs.insert(key, None);
            return None;
        }
        let slot = match self.allocator.allocate(size2(w as i32, h as i32)) {
            Some(slot) => slot,
            None => {
                self.clear();
                self.allocator.allocate(size2(w as i32, h as i32))?
            }
        };
        let (x0, y0) = (slot.rectangle.min.x as u32, slot.rectangle.min.y as u32);
        for row in 0..h {
            let from = (row * w) as usize;
            let to = ((y0 + row) * self.side + x0) as usize;
            self.pixels[to..to + w as usize].copy_from_slice(&image.data[from..from + w as usize]);
        }
        self.grow_dirty([x0, y0, x0 + w, y0 + h]);
        let glyph = AtlasGlyph {
            min: [x0 as u16, y0 as u16],
            size: [w as u16, h as u16],
            left: image.placement.left,
            top: image.placement.top,
        };
        self.glyphs.insert(key, Some(glyph));
        Some(glyph)
    }

    fn clear(&mut self) {
        self.allocator.clear();
        self.glyphs.clear();
        self.pixels.fill(0);
        self.generation = self.generation.wrapping_add(1);
        self.dirty = Some([0, 0, self.side, self.side]);
    }

    fn grow_dirty(&mut self, rect: [u32; 4]) {
        self.dirty = Some(match self.dirty {
            None => rect,
            Some(d) => [
                d[0].min(rect[0]),
                d[1].min(rect[1]),
                d[2].max(rect[2]),
                d[3].max(rect[3]),
            ],
        });
    }
}

/// Snaps a physical x to its subpixel bin: the integer pixel and which quarter of it.
pub(crate) fn subpixel(x: f32) -> (f32, u8) {
    let floor = x.floor();
    (floor, ((x - floor) * SUBPIXEL) as u8 % SUBPIXEL as u8)
}
