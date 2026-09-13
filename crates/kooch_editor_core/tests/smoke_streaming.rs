//! Editor smoke test (CI-friendly, headless).

use kooch_core::app::App;
use kooch_core::plugin::CorePlugin;
use kooch_core::stage::Stage;
use kooch_ecs::EcsPlugin;
use kooch_editor_core::editor_camera::{
    EditorCameraController, register_ephemeral_markers_system, spawn_editor_camera_system,
};
use kooch_world::{ChunkManager, WorldStreamingPlugin};

fn smoke_app() -> App {
    let mut app = App::new();
    app.add_plugin(CorePlugin);
    app.add_plugin(EcsPlugin);
    app.add_plugin(WorldStreamingPlugin);
    app.insert_resource(EditorCameraController::default());
    // Same Startup ordering as `EditorPlugin`: register ephemerals
    // first, then spawn the camera entity (which now carries
    // StreamingFocus by default — issue #362).
    app.add_system(Stage::Startup, register_ephemeral_markers_system);
    app.add_system(Stage::Startup, spawn_editor_camera_system);
    app
}

#[test]
fn editor_camera_drives_chunk_streaming_within_60_frames() {
    let mut app = smoke_app();

    app.schedule.run_startup(&mut app.resources);

    // Run the regular frame schedule. 60 frames is the issue's stated ceiling.
    let mut max_pending = 0;
    let mut max_loaded = 0;
    for frame in 1..=60 {
        app.schedule.run_frame_stages(&mut app.resources);
        let m = app
            .resources
            .get::<ChunkManager>()
            .expect("ChunkManager missing");
        max_pending = max_pending.max(m.pending_load_count());
        max_loaded = max_loaded.max(m.loaded_count());
        if max_pending > 0 || max_loaded > 0 {
            // First success short-circuits — proves the chain is live.
            return;
        }
        let _ = frame;
    }

    panic!(
        "no chunk activity within 60 frames — streaming chain looks broken (max_pending={max_pending} max_loaded={max_loaded})"
    );
}
