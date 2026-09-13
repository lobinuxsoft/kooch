//! [`FocusCacheState`]: the chunk each [`StreamingFocus`](super::focus::StreamingFocus) occupied
//! per LOD, so activation skips when nobody crossed a boundary — without it a still camera queued
//! millions of entries (#315).

use std::collections::{HashMap, HashSet};

use glam::{DVec3, IVec3};
use kooch_ecs::entity::Entity;

use crate::chunk::BASE_CHUNK_SIZE_METERS;

/// World-position of a focus paired with its entity. Free function
/// helper produces these from an ECS query in `activation`; the cache
/// itself only consumes them.
pub type FocusPosition = (Entity, DVec3);

/// Last chunk per focus and LOD, the `Vec` parallel to `LodRingConfig.rings`;
/// `IVec3::splat(i32::MIN)` means never observed.
#[derive(Default, Debug, Clone)]
pub struct FocusCacheState {
    last_chunks: HashMap<Entity, Vec<IVec3>>,
}

impl FocusCacheState {
    /// The `(entity, lod)` pairs that crossed a boundary, updating the cache in place so a caller
    /// cannot drift. Missing focuses are not purged here — see [`Self::purge_stale`].
    pub fn dirty_pairs(&mut self, focuses: &[FocusPosition], lod_count: u8) -> Vec<DirtyFocusLod> {
        let mut dirty = Vec::new();
        for (entity, pos) in focuses {
            let entry = self
                .last_chunks
                .entry(*entity)
                .or_insert_with(|| vec![IVec3::splat(i32::MIN); lod_count as usize]);
            if entry.len() != lod_count as usize {
                entry.resize(lod_count as usize, IVec3::splat(i32::MIN));
            }
            for lod in 0..lod_count {
                let chunk_size = BASE_CHUNK_SIZE_METERS * (1u64 << lod) as f64;
                let current = IVec3::new(
                    (pos.x / chunk_size).floor() as i32,
                    (pos.y / chunk_size).floor() as i32,
                    (pos.z / chunk_size).floor() as i32,
                );
                let last = entry[lod as usize];
                if current != last {
                    dirty.push(DirtyFocusLod {
                        entity: *entity,
                        lod,
                        previous: last,
                        current,
                    });
                    entry[lod as usize] = current;
                }
            }
        }
        dirty
    }

    /// Drop cached entries for entities that no longer appear in the
    /// active focus list. Call once per tick after `dirty_pairs` to
    /// keep the map bounded as focuses spawn and despawn.
    pub fn purge_stale(&mut self, current_focuses: &[FocusPosition]) {
        let alive: HashSet<Entity> = current_focuses.iter().map(|(e, _)| *e).collect();
        self.last_chunks.retain(|e, _| alive.contains(e));
    }

    #[cfg(test)]
    /// Test/debug accessor: number of focus entities currently tracked.
    pub fn tracked_count(&self) -> usize {
        self.last_chunks.len()
    }
}

/// A pair that changed chunk; `previous == IVec3::splat(i32::MIN)` is a first sighting, where every
/// chunk in range is new.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirtyFocusLod {
    pub entity: Entity,
    pub lod: u8,
    pub previous: IVec3,
    pub current: IVec3,
}

impl DirtyFocusLod {
    #[cfg(test)]
    /// Returns `true` when this dirty entry comes from a never-seen
    /// entity-lod pair (no previous position to diff against).
    pub fn is_first_seen(&self) -> bool {
        self.previous.x == i32::MIN
    }
}

#[cfg(test)]
mod tests;
