use std::collections::HashSet;

use glam::DVec3;
use ome_core::coord::ActiveOrigin;
use ome_core::resource::Resources;
use ome_ecs::entity::Entity;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::query::Query;

use crate::chunk::ChunkId;
use crate::focus::StreamingFocus;
use crate::focus_cache::{FocusCacheState, FocusPosition};
use crate::lod::LodRingConfig;
use crate::manager::ChunkManager;

use super::helpers::{chunk_priority, chunks_within_sphere};

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
