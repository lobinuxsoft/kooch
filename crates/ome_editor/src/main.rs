//! Launcher hub for Oh My Engine projects.
//!
//! Opens the launch screen where users can create, open, or select
//! recent projects. Each project is a Rust crate that compiles and
//! runs as its own binary with the editor embedded.
//!
//! Run with: cargo run -p ome_editor

use ome_core::prelude::*;
use ome_ecs::EcsPlugin;
use ome_editor_core::EditorPlugin;
use ome_render::plugin::AssetPlugin;
use ome_window::WindowPlugin;
use ome_world::WorldStreamingPlugin;

/// Path to the engine repository root (resolved at compile time).
///
/// `CARGO_MANIFEST_DIR` points to `crates/ome_editor/`, so we go two
/// levels up to reach the workspace root.
fn engine_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("ome_editor must live inside <engine_root>/crates/ome_editor")
        .to_path_buf()
}

/// Startup system that tells the editor where the engine source lives,
/// so that `create_project()` can generate valid `Cargo.toml` paths.
///
/// Honours the `OME_EDITOR_AUTO_OPEN` env var — when set to a project
/// directory, the editor opens it on startup so smoke / visual
/// verification runs (Issue #369 audit) can drive the streaming chain
/// without manual UI clicks. No-op when the var is unset.
fn set_engine_root(resources: &mut Resources) {
    if let Some(ps) = resources.get_mut::<ome_editor_core::ProjectState>() {
        ps.engine_root = Some(engine_root());
    }

    if let Ok(env_path) = std::env::var("OME_EDITOR_AUTO_OPEN") {
        let path = std::path::PathBuf::from(env_path);
        if path.exists() {
            auto_open_project(resources, &path);
        } else {
            tracing::warn!(
                path = %path.display(),
                "OME_EDITOR_AUTO_OPEN: project path does not exist",
            );
        }
    }
}

fn auto_open_project(resources: &mut Resources, path: &std::path::Path) {
    tracing::info!(path = %path.display(), "OME_EDITOR_AUTO_OPEN: opening project");
    let Some(mut ps) = resources.remove::<ome_editor_core::ProjectState>() else {
        return;
    };
    match ps.open_project(path) {
        Ok(()) => {
            let title = ps
                .active_project
                .as_ref()
                .map(|p| p.manifest.name.clone())
                .unwrap_or_default();
            tracing::info!(title, "OME_EDITOR_AUTO_OPEN: project opened");
        }
        Err(e) => {
            tracing::warn!(error = %e, "OME_EDITOR_AUTO_OPEN: open_project failed");
        }
    }
    resources.insert(ps);
    // NOTE: scene loading is owned by `actions::handle_open_project`
    // which is private. The streaming chain only needs `project_loaded
    // = true` + the editor camera's `StreamingFocus`, both of which the
    // open above already gives us — chunks activate without any
    // ECS-side SDFs in the scene file.
}

fn main() {
    ome_core::init_tracing();

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugin(WindowPlugin {
        title: "Oh My Engine".into(),
        width: 1280,
        height: 720,
    });
    app.add_plugin(EcsPlugin);
    app.add_plugin(AssetPlugin::default());
    app.add_plugin(WorldStreamingPlugin);
    app.add_plugin(EditorPlugin);
    app.add_system(Stage::Startup, set_engine_root);
    app.run();
}
