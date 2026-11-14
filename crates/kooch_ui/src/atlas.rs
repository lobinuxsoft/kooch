//! Rasterized glyphs and decoded images, packed into one RGBA8 atlas the draw samples: one
//! texture, one instanced draw (#96, #1708).

use std::collections::HashMap;
use std::time::SystemTime;

use kooch_core::Guid;

use etagere::{Allocation, BucketedAtlasAllocator, size2};
use swash::FontRef;
use swash::scale::{Render, Source, StrikeWith};
use swash::zeno::{Format, Vector};

mod animations;
mod pages;
mod vectors;

pub(crate) use pages::PAGE_ABOVE;

/// Fractions of a pixel a glyph's x is snapped to: four is where more stops being visible.
const SUBPIXEL: f32 = 4.0;

/// The atlas doubles up to this before it clears instead: 64 MiB of RGBA8 at the top.
const MAX_SIDE: u32 = 4096;
/// The largest size class: a raster whose longer side is half the atlas.
pub(crate) const TOP_CLASS: u8 = 8;

/// The size class of `shown` physical pixels: 1 holds a longer side up to 16, each next one twice
/// that, up to [`TOP_CLASS`]. 0 is an image drawn at its own size, or a size not known yet.
pub(crate) fn size_class(shown: [u32; 2]) -> u8 {
    match shown[0].max(shown[1]) {
        0 => 0,
        side => {
            (side
                .clamp(16, class_side(TOP_CLASS))
                .next_power_of_two()
                .trailing_zeros()
                - 3) as u8
        }
    }
}

/// The longer side of a raster of `class`.
pub(crate) fn class_side(class: u8) -> u32 {
    1 << (u32::from(class.clamp(1, TOP_CLASS)) + 3)
}

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

/// An image's texels in the atlas, `[x0, y0, x1, y1]`, and the file's mtime it was read at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtlasImage {
    pub rect: [u32; 4],
    pub modified: SystemTime,
    /// Texels per unit of the file, per axis: 1 for a PNG; an SVG is drawn at the size it shows.
    pub density: [f32; 2],
    /// Drawn by Vello into the vector atlas, not the RGBA8 one (#1865).
    pub vector: bool,
    /// In the page atlas, which has no CPU copy (#1893).
    pub page: bool,
}

/// The atlas: RGBA8 pixels, what is where, and what changed since the last upload.
pub struct GlyphAtlas {
    allocator: BucketedAtlasAllocator,
    side: u32,
    pixels: Vec<u8>,
    /// `None`: a glyph with no ink (a space). Cached too, so it is not rasterized every frame.
    glyphs: HashMap<GlyphKey, Option<AtlasGlyph>>,
    /// By asset and size class: an SVG is drawn once per class it shows at.
    images: HashMap<(Guid, u8), (etagere::AllocId, AtlasImage)>,
    /// Bounds of what was written since [`take_dirty`](Self::take_dirty), `[x0, y0, x1, y1]`.
    dirty: Option<[u32; 4]>,
    /// Bumped when a full atlas is cleared: every quad built before points at freed space.
    generation: u32,
    vectors: vectors::VectorSlots,
    /// Each image's own slice, read from its `.meta` when packed.
    slices: HashMap<Guid, [f32; 4]>,
    animations: animations::Animations,
    pages: pages::Pages,
}

impl Default for GlyphAtlas {
    fn default() -> Self {
        Self::new(1024)
    }
}

impl GlyphAtlas {
    pub fn new(side: u32) -> Self {
        Self {
            allocator: BucketedAtlasAllocator::new(size2(side as i32, side as i32)),
            side,
            pixels: vec![0; (side * side * 4) as usize],
            glyphs: HashMap::new(),
            images: HashMap::new(),
            dirty: None,
            generation: 0,
            vectors: vectors::VectorSlots::default(),
            slices: HashMap::new(),
            animations: animations::Animations::default(),
            pages: pages::Pages::default(),
        }
    }

    /// An empty atlas following one of `generation`, so every tree rastered into that one redraws.
    pub fn after(generation: u32) -> Self {
        Self {
            generation: generation.wrapping_add(1),
            ..Self::default()
        }
    }

    /// The slice `guid`'s `.meta` gave it, zero until it is packed.
    pub fn slice(&self, guid: Guid) -> [f32; 4] {
        self.slices.get(&guid).copied().unwrap_or_default()
    }

    pub(crate) fn set_slice(&mut self, guid: Guid, slice: [f32; 4]) {
        self.slices.insert(guid, slice);
    }

    pub fn side(&self) -> u32 {
        self.side
    }

    /// Side of the page atlas, 0 while nothing is packed there.
    pub fn page_side(&self) -> u32 {
        self.pages.side()
    }

    /// The rasters packed into the pages since the last call: the renderer is their only holder.
    pub fn take_page_uploads(&mut self) -> Vec<kooch_ecs::AtlasUpload> {
        self.pages.take_uploads()
    }

    /// RGBA8, row by row.
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

    /// The glyph's slot, rasterizing it on first use.
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
        let slot = self.allocate(w, h)?;
        let (x0, y0) = (slot.rectangle.min.x as u32, slot.rectangle.min.y as u32);
        for row in 0..h {
            for column in 0..w {
                let coverage = image.data[(row * w + column) as usize];
                let to = (((y0 + row) * self.side + x0 + column) * 4) as usize;
                self.pixels[to..to + 4].fill(coverage);
            }
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

    /// The mtime the raster of `guid` in `class` was read at, if there is one.
    /// `guid`'s raster, the one every size class shares.
    pub(crate) fn raster(&self, guid: Guid) -> Option<AtlasImage> {
        self.images.get(&(guid, 0)).map(|(_, image)| *image)
    }

    pub(crate) fn packed(&self, guid: Guid, class: u8) -> Option<SystemTime> {
        self.images
            .get(&(guid, class))
            .map(|(_, image)| image.modified)
            .or_else(|| self.vectors.get(guid, class).map(|image| image.modified))
    }

    /// What draws `guid` at `shown` physical pixels: its own size class, its only raster, or the
    /// nearest class packed so far.
    pub fn image(&self, guid: Guid, shown: [u32; 2]) -> Option<AtlasImage> {
        let class = size_class(shown);
        let get = |class: u8| {
            let raster = self.images.get(&(guid, class)).map(|(_, image)| *image);
            raster.or_else(|| self.vectors.get(guid, class))
        };
        get(class).or_else(|| get(0)).or_else(|| {
            let above = class.saturating_add(1)..=TOP_CLASS;
            above.chain((1..class).rev()).find_map(get)
        })
    }

    /// Packs an RGBA8 image, replacing what `guid` had. Its edge is repeated one texel out, so
    /// filtering at the border does not pull in a neighbour.
    pub(crate) fn insert_image(
        &mut self,
        (guid, class): (Guid, u8),
        modified: SystemTime,
        (w, h): (u32, u32),
        rgba: &[u8],
        density: [f32; 2],
    ) -> Option<AtlasImage> {
        if w == 0 || h == 0 {
            return None;
        }
        self.evict((guid, class));
        let slot = self.allocate(w + 2, h + 2)?;
        let (x0, y0) = (slot.rectangle.min.x as u32, slot.rectangle.min.y as u32);
        self.blit([x0, y0], (w, h), rgba);
        let image = AtlasImage {
            rect: [x0 + 1, y0 + 1, x0 + 1 + w, y0 + 1 + h],
            modified,
            density,
            vector: false,
            page: false,
        };
        self.images.insert((guid, class), (slot.id, image));
        Some(image)
    }

    /// Packs a raster in the pages when it is large, in the atlas when not.
    pub(crate) fn insert_raster(
        &mut self,
        key: (Guid, u8),
        modified: SystemTime,
        (w, h): (u32, u32),
        rgba: &[u8],
        density: [f32; 2],
    ) -> Option<AtlasImage> {
        match w.max(h) > PAGE_ABOVE {
            true => self.insert_page(key, modified, (w, h), rgba, density),
            false => self.insert_image(key, modified, (w, h), rgba, density),
        }
    }

    /// Packs an RGBA8 image in the pages, replacing what `guid` had. When they are full they are
    /// cleared, as the atlas is at its top.
    pub(crate) fn insert_page(
        &mut self,
        (guid, class): (Guid, u8),
        modified: SystemTime,
        (w, h): (u32, u32),
        rgba: &[u8],
        density: [f32; 2],
    ) -> Option<AtlasImage> {
        if w == 0 || h == 0 {
            return None;
        }
        self.evict((guid, class));
        let slot = match self.pages.allocate(w + 2, h + 2) {
            Some(slot) => slot,
            None => {
                self.clear_pages();
                self.pages.allocate(w + 2, h + 2)?
            }
        };
        let (x0, y0) = (slot.rectangle.min.x as u32, slot.rectangle.min.y as u32);
        self.pages.write([x0, y0], (w, h), rgba);
        let image = AtlasImage {
            rect: [x0 + 1, y0 + 1, x0 + 1 + w, y0 + 1 + h],
            modified,
            density,
            vector: false,
            page: true,
        };
        self.images.insert((guid, class), (slot.id, image));
        Some(image)
    }

    fn evict(&mut self, key: (Guid, u8)) {
        match self.images.remove(&key) {
            Some((id, image)) if image.page => self.pages.deallocate(id),
            Some((id, _)) => self.allocator.deallocate(id),
            None => {}
        }
    }

    fn clear_pages(&mut self) {
        self.pages.clear();
        self.images.retain(|_, (_, image)| !image.page);
        self.generation = self.generation.wrapping_add(1);
    }

    /// A slot, doubling the atlas while it is under [`MAX_SIDE`]; at the top, clearing it. Clearing
    /// first would evict what the frame still shows, and it would be packed again next frame.
    fn allocate(&mut self, w: u32, h: u32) -> Option<Allocation> {
        if w > MAX_SIDE || h > MAX_SIDE {
            return None;
        }
        let size = size2(w as i32, h as i32);
        loop {
            if let Some(slot) = self.allocator.allocate(size) {
                return Some(slot);
            }
            if self.side >= MAX_SIDE {
                self.clear();
                return self.allocator.allocate(size);
            }
            self.grow(self.side * 2);
        }
    }

    /// Keeps every slot where it is: the quads built before still point at their texels.
    fn grow(&mut self, side: u32) {
        let mut pixels = vec![0; (side * side * 4) as usize];
        let row = (self.side * 4) as usize;
        for (y, from) in self.pixels.chunks_exact(row).enumerate() {
            let to = y * (side * 4) as usize;
            pixels[to..to + row].copy_from_slice(from);
        }
        self.allocator.grow(size2(side as i32, side as i32));
        self.pixels = pixels;
        self.side = side;
        self.dirty = Some([0, 0, side, side]);
    }

    fn clear(&mut self) {
        self.allocator.clear();
        self.glyphs.clear();
        // The pages have their own slots, still where the quads point.
        self.images.retain(|_, (_, image)| image.page);
        self.pixels.fill(0);
        self.generation = self.generation.wrapping_add(1);
        self.dirty = Some([0, 0, self.side, self.side]);
    }

    /// Writes `rgba` into the slot at `[x0, y0]`, its edge repeated one texel out.
    fn blit(&mut self, [x0, y0]: [u32; 2], (w, h): (u32, u32), rgba: &[u8]) {
        if w == 0 || h == 0 || rgba.len() < (w * h * 4) as usize {
            return;
        }
        for row in 0..h + 2 {
            let from_row = row.saturating_sub(1).min(h - 1);
            for column in 0..w + 2 {
                let from_column = column.saturating_sub(1).min(w - 1);
                let from = ((from_row * w + from_column) * 4) as usize;
                let to = (((y0 + row) * self.side + x0 + column) * 4) as usize;
                if let Some(texel) = self.pixels.get_mut(to..to + 4) {
                    texel.copy_from_slice(&rgba[from..from + 4]);
                }
            }
        }
        self.grow_dirty([x0, y0, x0 + w + 2, y0 + h + 2]);
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
