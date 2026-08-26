//! [`FocusCacheState`] — tracks the chunk-index each [`StreamingFocus`]
//! occupied at the last activation tick, per LOD level, so the
//! activation system can skip recomputing the desired set when no
//! focus has crossed a chunk boundary on any LOD.
//!
//! Without this cache the brute-force grid iteration runs on every
//! frame even for a stationary camera, accumulating millions of
//! redundant queue entries — the regression caught in PR #315 / #54.
//!
//! [`StreamingFocus`]: super::focus::StreamingFocus

use std::collections::{HashMap, HashSet};

use glam::{DVec3, IVec3};
use kooch_ecs::entity::Entity;

use crate::chunk::BASE_CHUNK_SIZE_METERS;

/// World-position of a focus paired with its entity. Free function
/// helper produces these from an ECS query in `activation`; the cache
/// itself only consumes them.
pub type FocusPosition = (Entity, DVec3);

/// Per-focus, per-LOD record of "the chunk index this focus was in
/// last time we ticked". Compared against the current position to
/// detect crossings.
///
/// Stored as a flat `HashMap<Entity, Vec<IVec3>>`, indexed first by
/// focus entity, then by LOD level (the vector parallels
/// `LodRingConfig.rings`). `IVec3::splat(i32::MIN)` is the sentinel
/// for "never observed" — treated as a full first-time enter.
#[derive(Default, Debug, Clone)]
pub struct FocusCacheState {
    last_chunks: HashMap<Entity, Vec<IVec3>>,
}

impl FocusCacheState {
    /// Compute which `(entity, lod)` pairs have crossed at least one
    /// chunk boundary since the last call. Returns the dirty pairs
    /// **and** updates the cache in place to the new positions — a
    /// caller treating the result as immutable doesn't risk drift
    /// across frames.
    ///
    /// `focuses` is the live focus list this tick; entities missing
    /// from the call are NOT purged here (use [`Self::purge_stale`]).
    /// `lod_count` is the number of LOD rings the activation cares
    /// about (typically `LodRingConfig::lod_count()`).
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

    /// Test/debug accessor: number of focus entities currently tracked.
    pub fn tracked_count(&self) -> usize {
        self.last_chunks.len()
    }
}

/// One `(entity, lod)` pair that has changed chunk since the last
/// activation. `previous == IVec3::splat(i32::MIN)` means this is the
/// first time the cache sees this entity at this LOD — the activation
/// system treats that as "all chunks in range are new enters".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirtyFocusLod {
    pub entity: Entity,
    pub lod: u8,
    pub previous: IVec3,
    pub current: IVec3,
}

impl DirtyFocusLod {
    /// Returns `true` when this dirty entry comes from a never-seen
    /// entity-lod pair (no previous position to diff against).
    pub fn is_first_seen(&self) -> bool {
        self.previous.x == i32::MIN
    }
}

#[cfg(test)]
mod tests;
