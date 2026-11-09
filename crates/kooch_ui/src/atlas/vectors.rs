//! The vector atlas (#1865): SVGs packed like images, but drawn by Vello on the GPU into a texture
//! of their own, so the CPU keeps only the slots and the files. A glyph upload never lands there.
// Without `vello` nothing is inserted, and the lookups find nothing.
#![cfg_attr(not(feature = "vello"), allow(dead_code))]

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::SystemTime;

use etagere::{AllocId, BucketedAtlasAllocator, size2};
use kooch_core::Guid;
use kooch_ecs::{UiVector, UiVectorSource};

use super::{AtlasImage, GlyphAtlas, MAX_SIDE};

/// Where the vector atlas starts; it doubles up to [`MAX_SIDE`] before it clears.
const FIRST_SIDE: u32 = 512;

pub(crate) struct VectorSlots {
    allocator: BucketedAtlasAllocator,
    side: u32,
    entries: HashMap<(Guid, u8), (AllocId, AtlasImage, u64)>,
    sources: Vec<UiVectorSource>,
}

impl Default for VectorSlots {
    fn default() -> Self {
        Self {
            allocator: BucketedAtlasAllocator::new(size2(FIRST_SIDE as i32, FIRST_SIDE as i32)),
            side: FIRST_SIDE,
            entries: HashMap::new(),
            sources: Vec::new(),
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
    /// A texel of empty margin keeps the filter off its neighbours.
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
        let vectors = &mut self.vectors;
        if let Some((id, _, _)) = vectors.entries.remove(&(guid, class)) {
            vectors.allocator.deallocate(id);
        }
        let size = size2((w + 2) as i32, (h + 2) as i32);
        let slot = loop {
            if let Some(slot) = vectors.allocator.allocate(size) {
                break slot;
            }
            if vectors.side >= MAX_SIDE {
                // Every quad built before points at freed space, as when the glyph atlas clears.
                vectors.allocator.clear();
                vectors.entries.clear();
                self.generation = self.generation.wrapping_add(1);
                break vectors.allocator.allocate(size)?;
            }
            vectors.side *= 2;
            let side = vectors.side as i32;
            vectors.allocator.grow(size2(side, side));
        };
        let (x0, y0) = (slot.rectangle.min.x as u32, slot.rectangle.min.y as u32);
        let image = AtlasImage {
            rect: [x0 + 1, y0 + 1, x0 + 1 + w, y0 + 1 + h],
            modified,
            density,
            vector: true,
        };
        let key = key_of(guid, class, modified);
        vectors.entries.insert((guid, class), (slot.id, image, key));
        vectors.sources.push(UiVectorSource { key, svg });
        Some(image)
    }

    /// Side of the vector atlas, 0 while it holds nothing.
    pub(crate) fn vector_side(&self) -> u32 {
        match self.vectors.entries.is_empty() {
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
            .collect();
        vectors.sort_unstable_by_key(|vector| vector.key);
        vectors
    }

    /// The files packed since the last call.
    pub(crate) fn take_sources(&mut self) -> Vec<UiVectorSource> {
        std::mem::take(&mut self.vectors.sources)
    }
}

/// One key per file version and size class: a changed file is a new source.
fn key_of(guid: Guid, class: u8, modified: SystemTime) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (guid, class, modified).hash(&mut hasher);
    hasher.finish()
}
