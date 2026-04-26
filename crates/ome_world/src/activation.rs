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

/// ECS-aware activation: extracts focuses from the world, then calls
/// the pure [`activate_chunks`].
pub fn activation_system(
    resources: &Resources,
    manager: &mut ChunkManager,
    config: &LodRingConfig,
) {
    let origin = resources
        .get::<ActiveOrigin>()
        .copied()
        .unwrap_or_default();

    let mut focuses: Vec<(DVec3, u8)> = Vec::new();
    let q = Query::<(Entity, &StreamingFocus, &GlobalTransform)>::new(resources);
    q.for_each(|(_, focus, gt)| {
        if !focus.active {
            return;
        }
        let (_, _, translation) = gt.matrix.to_scale_rotation_translation();
        let universe_pos = origin.coord().translated(translation.as_dvec3()).to_dvec3();
        focuses.push((universe_pos, focus.priority));
    });
    drop(q);

    activate_chunks(&focuses, manager, config);
}

/// Enumerate every chunk at `lod` whose AABB intersects the sphere
/// (center, radius). Iterates a tight grid bounding box in world coords
/// and runs an exact AABB-sphere test per cell.
fn chunks_within_sphere(center: DVec3, radius: f64, lod: u8) -> Vec<ChunkId> {
    let chunk_size = BASE_CHUNK_SIZE_METERS * (1u64 << lod) as f64;
    let radius_sq = radius * radius;

    let min_world = center - DVec3::splat(radius);
    let max_world = center + DVec3::splat(radius);
    let min_idx = (min_world / chunk_size).floor();
    let max_idx = (max_world / chunk_size).ceil();

    let mut out = Vec::new();
    for x in min_idx.x as i32..max_idx.x as i32 {
        for y in min_idx.y as i32..max_idx.y as i32 {
            for z in min_idx.z as i32..max_idx.z as i32 {
                let chunk_min =
                    DVec3::new(x as f64, y as f64, z as f64) * chunk_size;
                let chunk_max = chunk_min + DVec3::splat(chunk_size);
                let closest = center.clamp(chunk_min, chunk_max);
                if (center - closest).length_squared() <= radius_sq {
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
        m.process_queues(usize::MAX, 0);
        assert!(m.active.contains_key(&ChunkId::new(IVec3::ZERO, 0)));
    }

    #[test]
    fn idempotent_when_active_set_matches_desired() {
        let mut m = ChunkManager::new(1024);
        activate_chunks(&[(DVec3::ZERO, 0)], &mut m, &one_ring(0, 100.0));
        m.process_queues(usize::MAX, 0);
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
        m.process_queues(usize::MAX, 0);
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
        m.process_queues(usize::MAX, 0);
        // Compare against single-focus run — must be at most slightly
        // larger (single focus + a few extra chunks on the +x edge).
        let mut m_single = ChunkManager::new(1024);
        activate_chunks(&[(DVec3::ZERO, 0)], &mut m_single, &cfg);
        m_single.process_queues(usize::MAX, 0);
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
        m.process_queues(usize::MAX, 0);

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
        m.process_queues(1, 0);
        assert!(
            m.active.contains_key(&ChunkId::new(IVec3::new(3, 0, 0), 0)),
            "first-popped chunk should be the one containing the focus, got {:?}",
            m.active.keys().collect::<Vec<_>>()
        );
    }
}
