//! Integration test (CPU-only) — confirms the streaming pipeline is live
//! end-to-end when an entity carries `StreamingFocus` + `GlobalTransform`.
//!
//! Mirrors the editor's runtime wiring without touching any GPU surface:
//! the editor adds `WorldStreamingPlugin` then `EditorPlugin` spawns the
//! camera with `StreamingFocus::default()`. This test asserts that the
//! activation system queues at least one chunk load when those two
//! pieces are present.
//!
//! Regression target: prior to issue #362 the editor loaded
//! `WorldStreamingPlugin` but no entity carried a focus, so the
//! activation system short-circuited to a no-op and the chunk pool
//! stayed cold regardless of camera position.

use glam::Mat4;

use ome_core::app::App;
use ome_core::plugin::CorePlugin;
use ome_ecs::EcsPlugin;
use ome_ecs::commands::Commands;
use ome_ecs::hierarchy::GlobalTransform;
use ome_world::{
    ChunkManager, LodRingConfig, StreamingFocus, WorldStreamingPlugin,
    plugin::world_streaming_system,
};

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugin(CorePlugin);
    app.add_plugin(EcsPlugin);
    app.add_plugin(WorldStreamingPlugin);
    // Drive Startup so the ECS plugin registers GlobalTransform and the
    // streaming plugin registers StreamingFocus before any spawn runs.
    app.schedule.run_startup(&mut app.resources);
    app
}

fn spawn_focus_at_origin(app: &mut App) {
    let mut commands = app
        .resources
        .remove::<Commands>()
        .expect("Commands resource missing");
    commands
        .spawn(&mut app.resources)
        .insert(StreamingFocus::default())
        .insert(GlobalTransform {
            matrix: Mat4::IDENTITY,
        });
    commands.apply(&mut app.resources);
    app.resources.insert(commands);
}

#[test]
fn focus_at_origin_queues_chunk_loads() {
    let mut app = build_app();
    spawn_focus_at_origin(&mut app);

    // One activation tick is enough — the cache flags the focus as
    // never-seen-before, so it walks the LOD ring and queues loads.
    world_streaming_system(&mut app.resources);

    let manager = app
        .resources
        .get::<ChunkManager>()
        .expect("ChunkManager resource missing");

    // Default LodRingConfig is one LOD-0 ring × 256 m, BASE chunk size
    // = 64 m. Either form of progress is acceptable: queued loads OR
    // drained loaded chunks.
    assert!(
        manager.pending_load_count() > 0 || manager.loaded_count() > 0,
        "expected activation to enqueue or load chunks, got pending={} loaded={}",
        manager.pending_load_count(),
        manager.loaded_count()
    );
}

#[test]
fn shrunk_radius_loads_fewer_chunks_than_default() {
    // Sanity check that the inspector slider would actually steer the
    // streaming horizon: cutting the radius shrinks the desired set.
    let mut tight_app = build_app();
    tight_app
        .resources
        .get_mut::<LodRingConfig>()
        .expect("LodRingConfig missing")
        .rings[0]
        .radius_meters = 64.0; // one chunk
    spawn_focus_at_origin(&mut tight_app);
    world_streaming_system(&mut tight_app.resources);

    let mut wide_app = build_app();
    spawn_focus_at_origin(&mut wide_app);
    world_streaming_system(&mut wide_app.resources);

    let tight = tight_app.resources.get::<ChunkManager>().unwrap();
    let wide = wide_app.resources.get::<ChunkManager>().unwrap();
    let tight_total = tight.pending_load_count() + tight.loaded_count();
    let wide_total = wide.pending_load_count() + wide.loaded_count();
    assert!(
        tight_total < wide_total,
        "shrinking the ring should reduce the desired chunk set: tight={tight_total} wide={wide_total}"
    );
}
