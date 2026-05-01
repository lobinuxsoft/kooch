//! Pure-CPU round trip of the #363 streaming chain:
//! `request_load → process_queues → drain_pending_loads`.
//!
//! Pins the contract the renderer relies on without requiring a wgpu
//! device. The GPU half (insert_chunk → OmeAccel::live_chunk_count == 1)
//! is exercised by `ome_render/tests/ac_363_demo_scene_traversal.rs`
//! when a wgpu device is available.

use glam::IVec3;
use ome_bvh::IS_RAYMARCH;
use ome_world::{ChunkId, ChunkManager, ProceduralCitySource};

fn id(x: i32, y: i32, z: i32) -> ChunkId {
    ChunkId::new(IVec3::new(x, y, z), 0)
}

#[test]
fn request_load_then_drain_yields_populated_content() {
    let mut manager = ChunkManager::new(64 * 1024 * 1024);
    manager.register_content_source(Box::new(ProceduralCitySource::new(0xCAFEF00D)));

    manager.request_load(id(0, 0, 0), 1.0);
    let (loaded, _) = manager.process_queues(8, 0, None);
    assert_eq!(loaded, 1, "exactly one chunk should have transitioned to Loaded");
    assert_eq!(manager.loaded_count(), 1);

    let pending = manager.drain_pending_loads();
    assert_eq!(pending.len(), 1, "drain must yield the populated chunk");
    let (drained_id, content) = &pending[0];
    assert_eq!(*drained_id, id(0, 0, 0));
    assert!(!content.is_empty(), "ProceduralCitySource never produces empty chunks");
    assert_eq!(content.primitives.len(), content.leaf_aabbs.len());
    for leaf in &content.leaf_aabbs {
        assert_ne!(leaf.flags & IS_RAYMARCH, 0, "leaves must be tagged IS_RAYMARCH");
    }

    // Drain is destructive — a second drain is empty.
    assert!(manager.drain_pending_loads().is_empty());
}

#[test]
fn unload_emits_pending_unload_event() {
    let mut manager = ChunkManager::new(64 * 1024 * 1024);
    manager.register_content_source(Box::new(ProceduralCitySource::new(7)));

    manager.request_load(id(2, 0, -1), 1.0);
    manager.process_queues(8, 0, None);
    // Drain so the load flow is "fully consumed" by the renderer
    // before we evict — mirrors the steady-state path.
    let _ = manager.drain_pending_loads();

    manager.request_unload(id(2, 0, -1));
    manager.process_queues(0, 8, None);
    assert_eq!(manager.loaded_count(), 0);

    let unloads = manager.drain_pending_unloads();
    assert_eq!(unloads, vec![id(2, 0, -1)]);
}

#[test]
fn load_then_unload_in_one_tick_does_not_emit_unload() {
    // If a chunk loads + unloads inside the same `process_queues`
    // call (before the renderer drains), the GPU never saw it — the
    // unload event must NOT fire, otherwise the renderer would call
    // `remove_chunk` for a key that was never inserted.
    let mut manager = ChunkManager::new(64 * 1024 * 1024);
    manager.register_content_source(Box::new(ProceduralCitySource::new(11)));

    manager.request_load(id(5, 0, 5), 1.0);
    manager.process_queues(8, 0, None);
    // Don't drain — simulate the renderer skipping a frame.
    manager.request_unload(id(5, 0, 5));
    manager.process_queues(0, 8, None);

    assert!(
        manager.drain_pending_unloads().is_empty(),
        "chunk that never reached the GPU must not emit an unload event",
    );
    assert!(
        manager.drain_pending_loads().is_empty(),
        "the load must have been cancelled by the unload",
    );
}

#[test]
fn churn_100_cycles_keeps_pending_buffers_bounded() {
    // Camera-movement regression for #369 audit: each frame we ingest a
    // new chunk on the leading edge and evict the one that just fell
    // off the trailing edge. The pending_loads / pending_unloads
    // buffers MUST drain to zero between frames once a (drain → drain)
    // call pair runs — otherwise the streaming layer accumulates
    // unbounded CPU memory while the editor moves around.
    const WINDOW: i32 = 8;
    let mut manager = ChunkManager::new(64 * 1024 * 1024);
    manager.register_content_source(Box::new(ProceduralCitySource::new(2026)));

    // Warm up: load `WINDOW` distinct chunks.
    for x in 0..WINDOW {
        manager.request_load(id(x, 0, 0), x as f32);
    }
    manager.process_queues(WINDOW as usize, 0, None);
    let _ = manager.drain_pending_loads();
    assert_eq!(manager.loaded_count(), WINDOW as usize);

    let mut max_pending_loads = 0usize;
    let mut max_pending_unloads = 0usize;

    // Steady-state cycle: each iteration models a single chunk-side
    // worth of camera movement. Load the leading-edge chunk, evict
    // the trailing-edge chunk (the OLDEST one the window left behind).
    for cycle in 0..100i32 {
        let lead = WINDOW + cycle;
        let trail = cycle;
        manager.request_load(id(lead, 0, 0), lead as f32);
        manager.request_unload(id(trail, 0, 0));
        manager.process_queues(WINDOW as usize, WINDOW as usize, None);
        let loads = manager.drain_pending_loads();
        let unloads = manager.drain_pending_unloads();
        max_pending_loads = max_pending_loads.max(loads.len());
        max_pending_unloads = max_pending_unloads.max(unloads.len());
        assert!(
            manager.drain_pending_loads().is_empty(),
            "cycle {cycle}: drain must zero pending_loads",
        );
        assert!(
            manager.drain_pending_unloads().is_empty(),
            "cycle {cycle}: drain must zero pending_unloads",
        );
    }

    // After 100 cycles the active set must still be bounded by the
    // window — no leak in `active`. Accumulation past `WINDOW` would
    // mean unloads aren't propagating.
    assert_eq!(
        manager.loaded_count(),
        WINDOW as usize,
        "active count drifted: loaded={} should equal WINDOW={WINDOW}",
        manager.loaded_count(),
    );
    assert!(max_pending_loads <= 1, "max pending loads {max_pending_loads}");
    assert!(max_pending_unloads <= 1, "max pending unloads {max_pending_unloads}");
}

#[test]
fn populate_is_seed_stable_across_manager_instances() {
    // AC6 of #360 (TLAS topology byte-identical under reordered loads)
    // requires the content the streaming layer hands to OmeAccel to be
    // a pure function of `(seed, chunk_id)`. Re-instantiating the
    // manager + source must produce the same primitives.
    let chunks = [id(0, 0, 0), id(1, 0, 0), id(0, 0, 1)];
    let collect = || {
        let mut m = ChunkManager::new(1024 * 1024);
        m.register_content_source(Box::new(ProceduralCitySource::new(12345)));
        for c in &chunks {
            m.request_load(*c, 1.0);
        }
        m.process_queues(8, 0, None);
        m.drain_pending_loads()
    };
    let mut a = collect();
    let mut b = collect();
    a.sort_by_key(|(id, _)| (id.coords.x, id.coords.y, id.coords.z));
    b.sort_by_key(|(id, _)| (id.coords.x, id.coords.y, id.coords.z));
    assert_eq!(a.len(), b.len());
    for ((id_a, c_a), (id_b, c_b)) in a.iter().zip(b.iter()) {
        assert_eq!(id_a, id_b);
        assert_eq!(c_a.primitives, c_b.primitives);
        assert_eq!(c_a.leaf_aabbs.len(), c_b.leaf_aabbs.len());
    }
}
