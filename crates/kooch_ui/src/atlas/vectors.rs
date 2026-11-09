//! The vector atlas (#1865): SVGs packed like images, but drawn by Vello on the GPU into a texture
//! of their own, so the CPU keeps only the slots and the files. A glyph upload never lands there.
// Without `vello` nothing is inserted, and the lookups find nothing.
#![cfg_attr(not(feature = "vello"), allow(dead_code))]

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::SystemTime;

use etagere::{AllocId, BucketedAtlasAllocator, size2};
use kooch_core::Guid;
use kooch_ecs::{UiFontSource, UiVector, UiVectorSource, UiVectorText};

use super::{AtlasImage, GlyphAtlas, MAX_SIDE};

/// Where the vector atlas starts; it doubles up to [`MAX_SIDE`] before it clears.
const FIRST_SIDE: u32 = 512;

pub(crate) struct VectorSlots {
    allocator: BucketedAtlasAllocator,
    side: u32,
    entries: HashMap<(Guid, u8), (AllocId, AtlasImage, u64)>,
    sources: Vec<UiVectorSource>,
    /// Text slots by content key, with how many trees hold each (#1880).
    texts: HashMap<u64, (AllocId, AtlasImage, u32)>,
    text_sources: Vec<UiVectorText>,
    /// Fonts handed over so far: each crosses once.
    fonts: HashSet<u64>,
    font_sources: Vec<UiFontSource>,
}

impl Default for VectorSlots {
    fn default() -> Self {
        Self {
            allocator: BucketedAtlasAllocator::new(size2(FIRST_SIDE as i32, FIRST_SIDE as i32)),
            side: FIRST_SIDE,
            entries: HashMap::new(),
            sources: Vec::new(),
            texts: HashMap::new(),
            text_sources: Vec::new(),
            fonts: HashSet::new(),
            font_sources: Vec::new(),
        }
    }
}

impl VectorSlots {
    pub(super) fn get(&self, guid: Guid, class: u8) -> Option<AtlasImage> {
        self.entries.get(&(guid, class)).map(|(_, image, _)| *image)
    }
}

impl GlyphAtlas {
    /// Reserves `w`×`h` texels for an SVG Vello draws there, replacing what `guid` had in `class`.
    pub(crate) fn insert_vector(
        &mut self,
        (guid, class): (Guid, u8),
        modified: SystemTime,
        (w, h): (u32, u32),
        density: [f32; 2],
        svg: Arc<[u8]>,
    ) -> Option<AtlasImage> {
        if w == 0 || h == 0 || w + 2 > MAX_SIDE || h + 2 > MAX_SIDE {
            return None;
        }
        if let Some((id, _, _)) = self.vectors.entries.remove(&(guid, class)) {
            self.vectors.allocator.deallocate(id);
        }
        let (id, rect) = self.vector_slot(w, h)?;
        let image = AtlasImage {
            rect,
            modified,
            density,
            vector: true,
        };
        let key = key_of(guid, class, modified);
        let vectors = &mut self.vectors;
        vectors.entries.insert((guid, class), (id, image, key));
        vectors.sources.push(UiVectorSource { key, svg });
        Some(image)
    }

    /// The slot of the text `key`, held once more. `text` builds what Vello draws, only when the
    /// slot is new; `fonts` are the files it reads, each handed over once.
    pub(crate) fn hold_text(
        &mut self,
        key: u64,
        (w, h): (u32, u32),
        text: impl FnOnce() -> UiVectorText,
        fonts: impl IntoIterator<Item = UiFontSource>,
    ) -> Option<AtlasImage> {
        if let Some((_, image, holders)) = self.vectors.texts.get_mut(&key) {
            *holders += 1;
            return Some(*image);
        }
        if w == 0 || h == 0 || w + 2 > MAX_SIDE || h + 2 > MAX_SIDE {
            return None;
        }
        let (id, rect) = self.vector_slot(w, h)?;
        let image = AtlasImage {
            rect,
            modified: SystemTime::UNIX_EPOCH,
            density: [1.0; 2],
            vector: true,
        };
        let vectors = &mut self.vectors;
        for font in fonts {
            if vectors.fonts.insert(font.id) {
                vectors.font_sources.push(font);
            }
        }
        vectors.texts.insert(key, (id, image, 1));
        vectors.text_sources.push(text());
        Some(image)
    }

    /// Gives back one hold of `key`; the last frees its slot.
    pub(crate) fn release_text(&mut self, key: u64) {
        let vectors = &mut self.vectors;
        let Some((id, _, holders)) = vectors.texts.get_mut(&key) else {
            return;
        };
        *holders -= 1;
        if *holders == 0 {
            let id = *id;
            vectors.texts.remove(&key);
            vectors.allocator.deallocate(id);
        }
    }

    /// Whether the font `id` was handed over already.
    pub(crate) fn has_font(&self, id: u64) -> bool {
        self.vectors.fonts.contains(&id)
    }

    /// `w`×`h` texels with a texel of empty margin, so the filter stays off the neighbours.
    fn vector_slot(&mut self, w: u32, h: u32) -> Option<(AllocId, [u32; 4])> {
        let vectors = &mut self.vectors;
        let size = size2((w + 2) as i32, (h + 2) as i32);
        let slot = loop {
            if let Some(slot) = vectors.allocator.allocate(size) {
                break slot;
            }
            if vectors.side >= MAX_SIDE {
                // Every quad built before points at freed space, as when the glyph atlas clears.
                vectors.allocator.clear();
                vectors.entries.clear();
                vectors.texts.clear();
                self.generation = self.generation.wrapping_add(1);
                break vectors.allocator.allocate(size)?;
            }
            vectors.side *= 2;
            let side = vectors.side as i32;
            vectors.allocator.grow(size2(side, side));
        };
        let (x0, y0) = (slot.rectangle.min.x as u32, slot.rectangle.min.y as u32);
        Some((slot.id, [x0 + 1, y0 + 1, x0 + 1 + w, y0 + 1 + h]))
    }

    /// Side of the vector atlas, 0 while it holds nothing.
    pub(crate) fn vector_side(&self) -> u32 {
        match self.vectors.entries.is_empty() && self.vectors.texts.is_empty() {
            true => 0,
            false => self.vectors.side,
        }
    }

    /// Every resident SVG, in a stable order so an unchanged atlas compares equal.
    pub(crate) fn vectors(&self) -> Vec<UiVector> {
        let mut vectors: Vec<UiVector> = self
            .vectors
            .entries
            .values()
            .map(|(_, image, key)| UiVector {
                key: *key,
                rect: image.rect,
            })
            .chain(
                self.vectors
                    .texts
                    .iter()
                    .map(|(key, (_, image, _))| UiVector {
                        key: *key,
                        rect: image.rect,
                    }),
            )
            .collect();
        vectors.sort_unstable_by_key(|vector| vector.key);
        vectors
    }

    /// The files, texts and fonts packed since the last call.
    pub(crate) fn take_sources(
        &mut self,
    ) -> (Vec<UiVectorSource>, Vec<UiVectorText>, Vec<UiFontSource>) {
        let vectors = &mut self.vectors;
        (
            std::mem::take(&mut vectors.sources),
            std::mem::take(&mut vectors.text_sources),
            std::mem::take(&mut vectors.font_sources),
        )
    }
}

/// One key per file version and size class: a changed file is a new source.
fn key_of(guid: Guid, class: u8, modified: SystemTime) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (guid, class, modified).hash(&mut hasher);
    hasher.finish()
}
