//! Launcher hub for Oh My Engine projects.
//!
//! Opens the launch screen where users can create, open, or select
//! recent projects. Each project is a Rust crate that compiles and
//! runs as its own binary with the editor embedded.
//!
//! Run with: cargo run --example editor --features editor

use ome_core::prelude::*;
use ome_ecs::EcsPlugin;
use ome_editor_core::EditorPlugin;
use ome_window::WindowPlugin;

/// Path to the engine repository root (resolved at compile time).
const ENGINE_ROOT: &str = env!("CARGO_MANIFEST_DIR");

/// Startup system that tells the editor where the engine source lives,
/// so that `create_project()` can generate valid `Cargo.toml` paths.
fn set_engine_root(resources: &mut Resources) {
    if let Some(ps) = resources.get_mut::<ome_editor_core::ProjectState>() {
        ps.engine_root = Some(std::path::PathBuf::from(ENGINE_ROOT));
    }
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
    app.add_plugin(EditorPlugin);
    app.add_system(Stage::Startup, set_engine_root);
    app.run();
}
