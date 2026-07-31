use glam::{DVec3, IVec3};
use kooch_ecs::entity::Entity;

use crate::chunk::ChunkId;
use crate::focus_cache::FocusCacheState;
use crate::lod::{LodRing, LodRingConfig};
use crate::manager::ChunkManager;

use super::{activate_chunks, activate_chunks_cached};

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
    m.process_queues(usize::MAX, 0);
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
    m.process_queues(usize::MAX, 0);

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
    m.process_queues(usize::MAX, 0);
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
    m.process_queues(usize::MAX, usize::MAX);
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
    m.process_queues(1, 0);
    assert!(
        m.active.contains_key(&ChunkId::new(IVec3::new(3, 0, 0), 0)),
        "first-popped chunk should be the one containing the focus, got {:?}",
        m.active.keys().collect::<Vec<_>>()
    );
}
