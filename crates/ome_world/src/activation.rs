//! Chunk activation — decides which chunks the manager should load /
//! unload based on the union of active [`StreamingFocus`] regions and
//! the global [`LodRingConfig`].
//!
//! Two layers:
//! - [`activate_chunks`] is the pure algorithm: takes focus positions
//!   in universe coords and updates the manager's queues. Trivial to
//!   unit-test without an ECS.
//! - [`activation_system`] is the ECS-aware wrapper: reads
//!   `ActiveOrigin` + iterates `(StreamingFocus, GlobalTransform)`
//!   entities, then delegates to the pure function.
//!
//! Coordinate convention: focus positions and chunk grid indices both
//! work in **absolute world / universe coordinates**, not the
//! simulation frame. A focus at `GlobalTransform.translation = (5,5,5)`
//! against `ActiveOrigin = (1000, 0, 0)` produces a focus universe
//! position of `(1005, 5, 5)`. This keeps the activation logic
//! correct across origin rebases without per-frame remapping of the
//! grid.

use std::collections::HashSet;

use glam::{DVec3, IVec3};
use ome_core::coord::ActiveOrigin;
use ome_core::resource::Resources;
use ome_ecs::entity::Entity;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::query::Query;

use crate::chunk::{BASE_CHUNK_SIZE_METERS, ChunkId};
use crate::focus::StreamingFocus;
use crate::focus_cache::{FocusCacheState, FocusPosition};
use crate::lod::LodRingConfig;
use crate::manager::ChunkManager;

/// Pure activation step.
///
/// `focuses[i] = (universe_position, priority)`. Walks the LOD ring
/// table, collects every chunk inside any focus's per-LOD radius, and
/// queues loads for new ones / unloads for chunks that fell out of
/// every focus.
pub fn activate_chunks(
    focuses: &[(DVec3, u8)],
    manager: &mut ChunkManager,
    config: &LodRingConfig,
) {
    let mut desired: HashSet<ChunkId> = HashSet::new();
    for ring in &config.rings {
        for (pos, _priority) in focuses {
            for id in chunks_within_sphere(*pos, ring.radius_meters as f64, ring.lod) {
                desired.insert(id);
            }
        }
    }

    // Loads: anything desired that isn't already active.
    for id in &desired {
        if !manager.active.contains_key(id) {
            let priority = chunk_priority(*id, focuses);
            manager.request_load(*id, priority);
        }
    }

    // Unloads: anything active that's no longer desired.
    let active_ids: Vec<ChunkId> = manager.active.keys().copied().collect();
    for id in active_ids {
        if !desired.contains(&id) {
            manager.request_unload(id);
        }
    }
}

/// Cached activation step. Same semantics as [`activate_chunks`] but
/// **skips entirely** when no focus has crossed a chunk boundary on
/// any LOD since the last call.
///
/// `focuses[i] = (entity, universe_position, priority)`. The cache is
/// updated in place each tick; stale entities (no longer in the focus
/// list) are purged automatically.
///
/// This is the normal entry point used by the schedule. The
/// uncached [`activate_chunks`] stays available as a pure helper for
/// tests / one-shot tooling.
pub fn activate_chunks_cached(
    focuses: &[(Entity, DVec3, u8)],
    cache: &mut FocusCacheState,
    manager: &mut ChunkManager,
    config: &LodRingConfig,
) {
    let positions: Vec<FocusPosition> =
        focuses.iter().map(|(e, p, _)| (*e, *p)).collect();
    let dirty = cache.dirty_pairs(&positions, config.lod_count());
    cache.purge_stale(&positions);

    if dirty.is_empty() {
        // No focus crossed a chunk boundary this tick — the desired
        // set is identical to last tick's, so nothing to enqueue.
        return;
    }

    let bare: Vec<(DVec3, u8)> = focuses.iter().map(|(_, p, pr)| (*p, *pr)).collect();
    activate_chunks(&bare, manager, config);
}

/// ECS-aware activation: extracts focuses from the world, then calls
/// [`activate_chunks_cached`] with the world-managed cache.
pub fn activation_system(
    resources: &Resources,
    cache: &mut FocusCacheState,
    manager: &mut ChunkManager,
    config: &LodRingConfig,
) {
    let origin = resources
        .get::<ActiveOrigin>()
        .copied()
        .unwrap_or_default();

    let mut focuses: Vec<(Entity, DVec3, u8)> = Vec::new();
    let q = Query::<(Entity, &StreamingFocus, &GlobalTransform)>::new(resources);
    q.for_each(|(entity, focus, gt)| {
        if !focus.active {
            return;
        }
        let (_, _, translation) = gt.matrix.to_scale_rotation_translation();
        let universe_pos = origin.coord().translated(translation.as_dvec3()).to_dvec3();
        focuses.push((entity, universe_pos, focus.priority));
    });
    drop(q);

    activate_chunks_cached(&focuses, cache, manager, config);
}

/// Enumerate every chunk at `lod` whose AABB intersects the sphere
/// `(center, radius)`. Iterates the bounding-box grid with **per-axis
/// early-out**: the squared distance from `center` to the closest
/// point of the current axis slab is accumulated outer → inner, so
/// whole `(y, z)` slices and whole `z` columns are skipped without
/// touching their cells.
///
/// Reduces the cell count tested from the cube bounding box (≈8r³ /
/// chunk³) to roughly the inscribed sphere (≈⁴⁄₃πr³ / chunk³) — about
/// 50 % fewer evaluations in the limit. The exact AABB-vs-sphere test
/// inside the inner loop stays identical, so the result set is
/// byte-identical to the old brute-force version.
fn chunks_within_sphere(center: DVec3, radius: f64, lod: u8) -> Vec<ChunkId> {
    let chunk_size = BASE_CHUNK_SIZE_METERS * (1u64 << lod) as f64;
    let radius_sq = radius * radius;

    let min_world = center - DVec3::splat(radius);
    let max_world = center + DVec3::splat(radius);
    let min_idx_x = (min_world.x / chunk_size).floor() as i32;
    let max_idx_x = (max_world.x / chunk_size).ceil() as i32;
    let min_idx_y = (min_world.y / chunk_size).floor() as i32;
    let max_idx_y = (max_world.y / chunk_size).ceil() as i32;
    let min_idx_z = (min_world.z / chunk_size).floor() as i32;
    let max_idx_z = (max_world.z / chunk_size).ceil() as i32;

    let mut out = Vec::new();
    for x in min_idx_x..max_idx_x {
        let cell_min_x = x as f64 * chunk_size;
        let cell_max_x = cell_min_x + chunk_size;
        let dx = center.x - center.x.clamp(cell_min_x, cell_max_x);
        let dx_sq = dx * dx;
        if dx_sq > radius_sq {
            // Whole y-z slice is outside the sphere — skip ~chunk_count² cells.
            continue;
        }

        for y in min_idx_y..max_idx_y {
            let cell_min_y = y as f64 * chunk_size;
            let cell_max_y = cell_min_y + chunk_size;
            let dy = center.y - center.y.clamp(cell_min_y, cell_max_y);
            let xy_sq = dx_sq + dy * dy;
            if xy_sq > radius_sq {
                // Whole z column is outside — skip ~chunk_count cells.
                continue;
            }

            for z in min_idx_z..max_idx_z {
                let cell_min_z = z as f64 * chunk_size;
                let cell_max_z = cell_min_z + chunk_size;
                let dz = center.z - center.z.clamp(cell_min_z, cell_max_z);
                if xy_sq + dz * dz <= radius_sq {
                    out.push(ChunkId::new(IVec3::new(x, y, z), lod));
                }
            }
        }
    }
    out
}

/// Squared distance from the chunk's centre to the closest focus.
/// Lower = higher priority for the load queue (closest pops first).
fn chunk_priority(id: ChunkId, focuses: &[(DVec3, u8)]) -> f32 {
    let chunk_size = id.size_meters();
    let centre = DVec3::new(
        id.coords.x as f64 + 0.5,
        id.coords.y as f64 + 0.5,
        id.coords.z as f64 + 0.5,
    ) * chunk_size;

    let mut min_d2 = f64::INFINITY;
    for (pos, _) in focuses {
        let d2 = (centre - *pos).length_squared();
        if d2 < min_d2 {
            min_d2 = d2;
        }
    }
    min_d2 as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lod::LodRing;

    fn one_ring(lod: u8, radius: f32) -> LodRingConfig {
        LodRingConfig {
            rings: vec![LodRing {
                lod,
                radius_meters: radius,
            }],
        }
    }

    fn entity(idx: u32) -> Entity {
        Entity::new(idx, 0)
    }

    #[test]
    fn no_focuses_loads_nothing() {
        let mut m = ChunkManager::new(1024);
        activate_chunks(&[], &mut m, &LodRingConfig::default());
        assert_eq!(m.pending_load_count(), 0);
    }

    #[test]
    fn single_focus_loads_local_ring() {
        // Focus at world origin, single LOD 0 ring of 100 m radius.
        // BASE_CHUNK_SIZE = 64 → expect chunks (0,0,0) and a small
        // shell around it.
        let mut m = ChunkManager::new(1024);
        activate_chunks(&[(DVec3::ZERO, 0)], &mut m, &one_ring(0, 100.0));
        assert!(m.pending_load_count() > 0);
        // The chunk containing the focus must be requested.
        m.process_queues(usize::MAX, 0, None);
        assert!(m.active.contains_key(&ChunkId::new(IVec3::ZERO, 0)));
    }

    #[test]
    fn idempotent_when_active_set_matches_desired() {
        let mut m = ChunkManager::new(1024);
        activate_chunks(&[(DVec3::ZERO, 0)], &mut m, &one_ring(0, 100.0));
        m.process_queues(usize::MAX, 0, None);
        let loaded_a = m.loaded_count();
        // Run again: nothing new to load, nothing to unload.
        activate_chunks(&[(DVec3::ZERO, 0)], &mut m, &one_ring(0, 100.0));
        assert_eq!(m.pending_load_count(), 0);
        assert_eq!(m.pending_unload_count(), 0);
        assert_eq!(m.loaded_count(), loaded_a);
    }

    #[test]
    fn focus_moves_away_unloads_old_chunks() {
        let mut m = ChunkManager::new(1024);
        let cfg = one_ring(0, 100.0);
        activate_chunks(&[(DVec3::ZERO, 0)], &mut m, &cfg);
        m.process_queues(usize::MAX, 0, None);
        let initial = m.loaded_count();
        assert!(initial > 0);

        // Move focus 10 km away — the original chunks fall out of range.
        activate_chunks(&[(DVec3::new(10_000.0, 0.0, 0.0), 0)], &mut m, &cfg);
        // At least all original chunks are queued for unload.
        assert!(m.pending_unload_count() >= initial);
    }

    #[test]
    fn two_overlapping_focuses_dedup_via_set() {
        // Two focuses 10 m apart, single LOD 0 ring of 100 m. Their
        // chunk sets overlap heavily; the union must not produce
        // duplicates in the load queue.
        let mut m = ChunkManager::new(1024);
        let cfg = one_ring(0, 100.0);
        activate_chunks(
            &[(DVec3::ZERO, 0), (DVec3::new(10.0, 0.0, 0.0), 0)],
            &mut m,
            &cfg,
        );
        m.process_queues(usize::MAX, 0, None);
        // Compare against single-focus run — must be at most slightly
        // larger (single focus + a few extra chunks on the +x edge).
        let mut m_single = ChunkManager::new(1024);
        activate_chunks(&[(DVec3::ZERO, 0)], &mut m_single, &cfg);
        m_single.process_queues(usize::MAX, 0, None);
        // Loaded count of two-focus run must NOT be ~2× the single-focus
        // run (which would mean we duplicated chunks).
        assert!(m.loaded_count() <= m_single.loaded_count() + 5);
    }

    #[test]
    fn higher_lod_rings_load_coarser_grid_chunks() {
        // LOD 0 ring 100 m + LOD 2 ring 1000 m: should load LOD 2
        // chunks (256 m side) covering the larger area.
        let mut m = ChunkManager::new(1024);
        let cfg = LodRingConfig {
            rings: vec![
                LodRing {
                    lod: 0,
                    radius_meters: 100.0,
                },
                LodRing {
                    lod: 2,
                    radius_meters: 1000.0,
                },
            ],
        };
        activate_chunks(&[(DVec3::ZERO, 0)], &mut m, &cfg);
        m.process_queues(usize::MAX, 0, None);

        let lod0_count = m
            .active
            .keys()
            .filter(|id| id.level == 0)
            .count();
        let lod2_count = m
            .active
            .keys()
            .filter(|id| id.level == 2)
            .count();
        assert!(lod0_count > 0, "expected at least one LOD 0 chunk");
        assert!(lod2_count > 0, "expected at least one LOD 2 chunk");
    }

    // -- cached-activation tests ----------------------------------------

    #[test]
    fn cached_first_call_loads_chunks() {
        let mut m = ChunkManager::new(1024);
        let mut cache = FocusCacheState::default();
        activate_chunks_cached(
            &[(entity(1), DVec3::ZERO, 0)],
            &mut cache,
            &mut m,
            &one_ring(0, 100.0),
        );
        // First call sees the focus as never-before-seen → activates.
        assert!(m.pending_load_count() > 0);
    }

    #[test]
    fn cached_stationary_second_call_skips_work() {
        let mut m = ChunkManager::new(1024);
        let mut cache = FocusCacheState::default();
        let cfg = one_ring(0, 100.0);
        let focuses = [(entity(1), DVec3::new(10.0, 10.0, 10.0), 0u8)];

        activate_chunks_cached(&focuses, &mut cache, &mut m, &cfg);
        m.process_queues(usize::MAX, 0, None);
        let loaded_after_first = m.loaded_count();
        let pending_after_first = m.pending_load_count();
        assert!(loaded_after_first > 0);
        assert_eq!(pending_after_first, 0, "first call should have drained");

        // Second call: same position. Cache reports no dirty pairs.
        activate_chunks_cached(&focuses, &mut cache, &mut m, &cfg);
        // Manager state is unchanged — no new pending loads, no unloads.
        assert_eq!(m.pending_load_count(), 0);
        assert_eq!(m.pending_unload_count(), 0);
        assert_eq!(m.loaded_count(), loaded_after_first);
    }

    #[test]
    fn cached_sub_chunk_movement_skips_work() {
        let mut m = ChunkManager::new(1024);
        let mut cache = FocusCacheState::default();
        let cfg = one_ring(0, 100.0);

        // First call at (10,10,10) — chunk (0,0,0).
        activate_chunks_cached(
            &[(entity(1), DVec3::new(10.0, 10.0, 10.0), 0)],
            &mut cache,
            &mut m,
            &cfg,
        );
        m.process_queues(usize::MAX, 0, None);

        // Second call: focus moved 30 m on x — still chunk (0,0,0) at LOD 0
        // (chunk size 64).
        activate_chunks_cached(
            &[(entity(1), DVec3::new(40.0, 10.0, 10.0), 0)],
            &mut cache,
            &mut m,
            &cfg,
        );
        // No work (still in same chunk).
        assert_eq!(m.pending_load_count(), 0);
        assert_eq!(m.pending_unload_count(), 0);
    }

    #[test]
    fn cached_chunk_boundary_cross_triggers_work() {
        let mut m = ChunkManager::new(1024);
        let mut cache = FocusCacheState::default();
        let cfg = one_ring(0, 100.0);

        activate_chunks_cached(
            &[(entity(1), DVec3::ZERO, 0)],
            &mut cache,
            &mut m,
            &cfg,
        );
        m.process_queues(usize::MAX, 0, None);
        let loaded_initial = m.loaded_count();

        // Move 100 m on x — crosses LOD-0 boundary at 64.
        activate_chunks_cached(
            &[(entity(1), DVec3::new(100.0, 0.0, 0.0), 0)],
            &mut cache,
            &mut m,
            &cfg,
        );
        // Some chunks newly desired (right of focus), some no longer
        // desired (left). Work was done.
        assert!(
            m.pending_load_count() > 0 || m.pending_unload_count() > 0,
            "boundary crossing must produce queue activity"
        );
        // After draining, total loaded should still be reasonable.
        m.process_queues(usize::MAX, usize::MAX, None);
        assert!(m.loaded_count() > 0);
        let _ = loaded_initial;
    }

    #[test]
    fn cached_purges_dropped_focus() {
        let mut m = ChunkManager::new(1024);
        let mut cache = FocusCacheState::default();
        let cfg = one_ring(0, 100.0);

        // Two focuses, then one despawns.
        activate_chunks_cached(
            &[
                (entity(1), DVec3::ZERO, 0),
                (entity(2), DVec3::new(500.0, 0.0, 0.0), 0),
            ],
            &mut cache,
            &mut m,
            &cfg,
        );
        assert_eq!(cache.tracked_count(), 2);

        // Tick again with only entity 1 — entity 2 is purged.
        activate_chunks_cached(
            &[(entity(1), DVec3::ZERO, 0)],
            &mut cache,
            &mut m,
            &cfg,
        );
        assert_eq!(cache.tracked_count(), 1);
    }

    // -- legacy activate_chunks tests (uncached path) -------------------

    #[test]
    fn closest_chunk_gets_lowest_priority() {
        // After a single activation pass, the closest chunk to the
        // focus must be at the top of the load queue.
        let mut m = ChunkManager::new(1024);
        // Focus at (210, 30, 30) — solidly inside chunk (3, 0, 0) at
        // LOD 0 (chunk covers [192, 256) × [0, 64) × [0, 64)). Picked
        // off the integer boundaries so neighbouring chunks aren't
        // tied for closest.
        activate_chunks(
            &[(DVec3::new(210.0, 30.0, 30.0), 0)],
            &mut m,
            &one_ring(0, 100.0),
        );
        m.process_queues(1, 0, None);
        assert!(
            m.active.contains_key(&ChunkId::new(IVec3::new(3, 0, 0), 0)),
            "first-popped chunk should be the one containing the focus, got {:?}",
            m.active.keys().collect::<Vec<_>>()
        );
    }
}
