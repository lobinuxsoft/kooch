//! Game runner — standalone process for play mode.
//!
//! Loads a `.ome_scene` file and runs the game loop without the editor
//! overlay. Launched by the editor's Play button as a child process.
//!
//! Run with: cargo run --example game_runner -- --scene <path>

use std::path::PathBuf;

use ome_core::prelude::*;
use ome_ecs::EcsPlugin;
use ome_window::WindowPlugin;

// ---------------------------------------------------------------------------
// Scene loader
// ---------------------------------------------------------------------------

/// CLI argument holding the scene file path.
struct ScenePathArg(PathBuf);

fn load_scene_system(resources: &mut Resources) {
    let Some(arg) = resources.remove::<ScenePathArg>() else {
        tracing::error!("no scene path provided");
        return;
    };

    match ome_ecs::SceneDocument::load(&arg.0) {
        Ok(doc) => {
            tracing::info!("loading scene: {}", doc.name);
            if let Err(e) = ome_ecs::sync_scene_to_ecs(&doc, resources) {
                tracing::error!("failed to sync scene to ECS: {e}");
            }
        }
        Err(e) => tracing::error!("failed to load scene '{}': {e}", arg.0.display()),
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    ome_core::init_tracing();

    let args: Vec<String> = std::env::args().collect();
    let scene_path = args
        .iter()
        .position(|a| a == "--scene")
        .and_then(|i| args.get(i + 1))
        .expect("usage: game_runner --scene <path>");

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugin(WindowPlugin {
        title: "OME Game".into(),
        width: 1280,
        height: 720,
    });
    app.add_plugin(EcsPlugin);
    app.insert_resource(ScenePathArg(PathBuf::from(scene_path)));
    app.add_system(Stage::Startup, load_scene_system);
    app.run();
}
