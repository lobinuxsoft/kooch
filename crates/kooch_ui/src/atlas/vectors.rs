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
use crate::crops::Crop;

/// A slot, the image it holds, its key and the file it draws.
type Slot = (AllocId, AtlasImage, u64, u64);

/// Where the vector atlas starts; it doubles up to [`MAX_SIDE`] before it clears.
const FIRST_SIDE: u32 = 512;

pub(crate) struct VectorSlots {
    allocator: BucketedAtlasAllocator,
    side: u32,
    entries: HashMap<(Guid, u8), Slot>,
    /// Parts of nodes past the largest class (#1880), kept only while wanted.
    crops: HashMap<(Guid, Crop), Slot>,
    /// How many slots draw each file: it is handed over when the first appears, and again after
    /// the last went, since the renderer lets go of a file nothing draws.
    files: HashMap<u64, u32>,
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
            crops: HashMap::new(),
            files: HashMap::new(),
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
        self.entries
            .get(&(guid, class))
            .map(|(_, image, _, _)| *image)
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
        if let Some(slot) = self.vectors.entries.remove(&(guid, class)) {
            self.vectors.free(slot);
        }
        let (id, rect) = self.vector_slot(w, h)?;
        let image = AtlasImage {
            rect,
            modified,
            density,
            vector: true,
            page: false,
        };
        let key = key_of((guid, class), modified);
        let file = key_of(guid, modified);
        let vectors = &mut self.vectors;
        vectors
            .entries
            .insert((guid, class), (id, image, key, file));
        vectors.hold_file(file, svg);
        Some(image)
    }

    /// The slot drawing `crop` of `guid`, if packed at `modified`.
    pub(crate) fn crop(&self, guid: Guid, crop: Crop) -> Option<AtlasImage> {
        let slot = self.vectors.crops.get(&(guid, crop));
        slot.map(|(_, image, _, _)| *image)
    }

    /// Reserves the texels of `crop` of an SVG, drawn at `crop.full`.
    pub(crate) fn insert_crop(
        &mut self,
        guid: Guid,
        crop: Crop,
        modified: SystemTime,
        svg: Arc<[u8]>,
    ) -> Option<AtlasImage> {
        let [w, h] = crop.size;
        if w == 0 || h == 0 || w + 2 > MAX_SIDE || h + 2 > MAX_SIDE {
            return None;
        }
        if let Some(slot) = self.vectors.crops.remove(&(guid, crop)) {
            self.vectors.free(slot);
        }
        let (id, rect) = self.vector_slot(w, h)?;
        let image = AtlasImage {
            rect,
            modified,
            density: [1.0; 2],
            vector: true,
            page: false,
        };
        let key = key_of((guid, crop), modified);
        let file = key_of(guid, modified);
        let vectors = &mut self.vectors;
        vectors.crops.insert((guid, crop), (id, image, key, file));
        vectors.hold_file(file, svg);
        Some(image)
    }

    /// Frees every crop not in `wanted`: a panned view leaves the old ones behind.
    pub(crate) fn keep_crops(&mut self, wanted: &HashSet<(Guid, Crop)>) {
        let vectors = &mut self.vectors;
        let gone: Vec<(Guid, Crop)> = vectors
            .crops
            .keys()
            .filter(|key| !wanted.contains(key))
            .copied()
            .collect();
        for key in gone {
            if let Some(slot) = vectors.crops.remove(&key) {
                vectors.free(slot);
            }
        }
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
            page: false,
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
                vectors.crops.clear();
                vectors.files.clear();
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
        let vectors = &self.vectors;
        match vectors.entries.is_empty() && vectors.crops.is_empty() && vectors.texts.is_empty() {
            true => 0,
            false => self.vectors.side,
        }
    }

    /// Every resident SVG, in a stable order so an unchanged atlas compares equal.
    pub(crate) fn vectors(&self) -> Vec<UiVector> {
        let whole = |(_, image, key, file): &Slot| UiVector {
            key: *key,
            rect: image.rect,
            source: *file,
            crop: [0.0; 4],
        };
        let part = |((_, crop), (_, image, key, file)): (&(Guid, Crop), &Slot)| UiVector {
            key: *key,
            rect: image.rect,
            source: *file,
            crop: [crop.full[0], crop.full[1], crop.from[0], crop.from[1]].map(|v| v as f32),
        };
        let text = |(key, (_, image, _)): (&u64, &(AllocId, AtlasImage, u32))| UiVector {
            key: *key,
            rect: image.rect,
            source: *key,
            crop: [0.0; 4],
        };
        let vectors = &self.vectors;
        let mut vectors: Vec<UiVector> = vectors
            .entries
            .values()
            .map(whole)
            .chain(vectors.crops.iter().map(part))
            .chain(vectors.texts.iter().map(text))
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

impl VectorSlots {
    fn free(&mut self, (id, _, _, file): Slot) {
        self.allocator.deallocate(id);
        if let Some(count) = self.files.get_mut(&file) {
            *count -= 1;
            if *count == 0 {
                self.files.remove(&file);
            }
        }
    }

    fn hold_file(&mut self, file: u64, svg: Arc<[u8]>) {
        let count = self.files.entry(file).or_insert(0);
        if *count == 0 {
            self.sources.push(UiVectorSource { key: file, svg });
        }
        *count += 1;
    }
}

/// One key per file version and what of it is drawn: a changed file is a new key.
fn key_of(what: impl Hash, modified: SystemTime) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (what, modified).hash(&mut hasher);
    hasher.finish()
}
