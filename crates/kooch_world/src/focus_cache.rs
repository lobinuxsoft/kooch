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
    pub fn dirty_pairs(
        &mut self,
        focuses: &[FocusPosition],
        lod_count: u8,
    ) -> Vec<DirtyFocusLod> {
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
mod tests {
    use super::*;

    fn entity(idx: u32) -> Entity {
        Entity::new(idx, 0)
    }

    #[test]
    fn first_call_reports_all_first_seen() {
        let mut cache = FocusCacheState::default();
        let focuses = vec![(entity(1), DVec3::ZERO)];
        let dirty = cache.dirty_pairs(&focuses, 4);
        assert_eq!(dirty.len(), 4); // one per LOD ring
        assert!(dirty.iter().all(|d| d.is_first_seen()));
        assert_eq!(cache.tracked_count(), 1);
    }

    #[test]
    fn stationary_focus_no_dirty() {
        let mut cache = FocusCacheState::default();
        let focuses = vec![(entity(1), DVec3::new(5.0, 5.0, 5.0))];
        // First call seeds the cache.
        let _ = cache.dirty_pairs(&focuses, 4);
        // Second call with same position: nothing dirty.
        let dirty = cache.dirty_pairs(&focuses, 4);
        assert_eq!(dirty.len(), 0);
    }

    #[test]
    fn movement_within_same_chunk_no_dirty() {
        let mut cache = FocusCacheState::default();
        // Chunk size at LOD 0 is 64 m; two points 10 m apart at the
        // same chunk index.
        let _ = cache.dirty_pairs(&[(entity(1), DVec3::new(10.0, 10.0, 10.0))], 4);
        let dirty = cache.dirty_pairs(&[(entity(1), DVec3::new(20.0, 30.0, 40.0))], 4);
        assert_eq!(dirty.len(), 0, "still chunk (0,0,0) at all LODs");
    }

    #[test]
    fn cross_chunk_boundary_marks_dirty_at_lod_0() {
        let mut cache = FocusCacheState::default();
        let _ = cache.dirty_pairs(&[(entity(1), DVec3::new(10.0, 10.0, 10.0))], 4);
        // Move past the LOD-0 boundary on x (64 m). Higher LODs (128,
        // 256, 512 m) still resolve to the same cell.
        let dirty = cache.dirty_pairs(&[(entity(1), DVec3::new(70.0, 10.0, 10.0))], 4);
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0].lod, 0);
        assert_eq!(dirty[0].current, IVec3::new(1, 0, 0));
        assert_eq!(dirty[0].previous, IVec3::ZERO);
        assert!(!dirty[0].is_first_seen());
    }

    #[test]
    fn long_jump_marks_all_lods_dirty() {
        let mut cache = FocusCacheState::default();
        let _ = cache.dirty_pairs(&[(entity(1), DVec3::ZERO)], 4);
        // Jump 10 km on x: crosses every LOD's boundary (64, 128, 256,
        // 512 m).
        let dirty = cache.dirty_pairs(&[(entity(1), DVec3::new(10_000.0, 0.0, 0.0))], 4);
        assert_eq!(dirty.len(), 4);
    }

    #[test]
    fn only_moved_focus_is_dirty() {
        let mut cache = FocusCacheState::default();
        let initial = vec![
            (entity(1), DVec3::new(10.0, 10.0, 10.0)),
            (entity(2), DVec3::new(500.0, 0.0, 0.0)),
        ];
        let _ = cache.dirty_pairs(&initial, 4);
        // Only entity 1 crosses a boundary; entity 2 stays put.
        let next = vec![
            (entity(1), DVec3::new(70.0, 10.0, 10.0)),
            (entity(2), DVec3::new(500.0, 0.0, 0.0)),
        ];
        let dirty = cache.dirty_pairs(&next, 4);
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0].entity, entity(1));
    }

    #[test]
    fn purge_stale_drops_missing_entities() {
        let mut cache = FocusCacheState::default();
        let _ = cache.dirty_pairs(
            &[
                (entity(1), DVec3::ZERO),
                (entity(2), DVec3::new(100.0, 0.0, 0.0)),
                (entity(3), DVec3::new(-100.0, 0.0, 0.0)),
            ],
            4,
        );
        assert_eq!(cache.tracked_count(), 3);
        // Only entity(2) survives.
        cache.purge_stale(&[(entity(2), DVec3::new(100.0, 0.0, 0.0))]);
        assert_eq!(cache.tracked_count(), 1);
    }

    #[test]
    fn lod_count_change_resizes_cache() {
        let mut cache = FocusCacheState::default();
        let _ = cache.dirty_pairs(&[(entity(1), DVec3::ZERO)], 4);
        // Simulate a LodRingConfig change at runtime — fewer rings.
        let dirty = cache.dirty_pairs(&[(entity(1), DVec3::ZERO)], 2);
        // Cache is resized; no false positives.
        assert_eq!(dirty.len(), 0);
    }

    #[test]
    fn negative_position_floors_correctly() {
        let mut cache = FocusCacheState::default();
        // -10 / 64 = -0.156 → floor = -1. Avoid the rust-default
        // truncate-toward-zero pitfall.
        let dirty = cache.dirty_pairs(&[(entity(1), DVec3::new(-10.0, 0.0, 0.0))], 1);
        assert_eq!(dirty[0].current.x, -1);
    }
}
