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

/// Resolves the engine's root directory at runtime, in this order:
///
/// 1. `OME_ENGINE_ROOT` env var if set — explicit override for
///    custom installations and CI.
/// 2. The directory containing the running executable, if it has a
///    sibling `assets/` (the production layout: editor binary plus
///    its assets shipped together).
/// 3. The first ancestor of the executable that contains `assets/`
///    (covers the dev layout where the binary is in
///    `target/release/` or `target/debug/` and the assets live at
///    the workspace root).
/// 4. Compile-time `CARGO_MANIFEST_DIR` walk — last-resort fallback
///    that only works for local source checkouts.
///
/// Panics if none of the above resolves to an existing directory:
/// the editor cannot run without locating its bundled assets.
fn engine_root() -> std::path::PathBuf {
    if let Ok(env) = std::env::var("OME_ENGINE_ROOT") {
        let p = std::path::PathBuf::from(env);
        if p.exists() {
            return p;
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if dir.join("assets").is_dir() {
                return dir.to_path_buf();
            }
            for ancestor in dir.ancestors().skip(1) {
                if ancestor.join("assets").is_dir() {
                    return ancestor.to_path_buf();
                }
            }
        }
    }

    let manifest_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf());
    if let Some(p) = manifest_root
        && p.join("assets").is_dir()
    {
        return p;
    }

    panic!(
        "engine_root could not be resolved: set OME_ENGINE_ROOT, ship assets/ next \
         to the executable, or run from a source checkout that contains assets/",
    );
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

/// Forces winit onto the X11 backend (XWayland) on Linux by clearing
/// `WAYLAND_DISPLAY` before the event loop is built.
///
/// Workaround for egui's IBus+Wayland IME bug (egui #7485): under a
/// Wayland session with IBus, a `TextEdit` accepts only one character
/// before silently dropping input until the widget is re-focused. It is
/// fixed upstream by egui #7983 (egui 0.35+); we are pinned to 0.34, so
/// until that bump lands every text field in the editor is unusable on
/// native Wayland. XWayland is not affected.
///
/// Opt back into native Wayland with `OME_FORCE_WAYLAND=1` (e.g. once
/// egui is bumped, or to test the native path). No-op on non-Linux and
/// on X11 sessions (where `WAYLAND_DISPLAY` is already absent).
fn force_x11_backend_if_needed() {
    if !cfg!(target_os = "linux") {
        return;
    }
    if std::env::var_os("OME_FORCE_WAYLAND").is_some() {
        return;
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        // SAFETY: called at the very top of `main`, before any threads
        // are spawned (tracing included) and before the winit event loop
        // reads the env — no concurrent env access can race this.
        unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
        // `init_tracing` has not run yet, so use stderr directly.
        eprintln!(
            "ome_editor: cleared WAYLAND_DISPLAY to force XWayland \
             (egui #7485 IME workaround); set OME_FORCE_WAYLAND=1 to keep native Wayland",
        );
    }
}

fn main() {
    force_x11_backend_if_needed();
    ome_core::init_tracing();

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugin(WindowPlugin {
        title: "Oh My Engine".into(),
        width: 1280,
        height: 720,
    });
    app.add_plugin(EcsPlugin);
    // Engine assets (demo glb, default sky textures, etc.) live in the
    // engine repo's `assets/` tree — resolved against an ABSOLUTE path
    // because the editor's cwd shifts when the user opens a project.
    app.add_plugin(AssetPlugin::new().with_root(engine_root().join("assets")));
    app.add_plugin(WorldStreamingPlugin);
    app.add_plugin(EditorPlugin);
    app.add_system(Stage::Startup, set_engine_root);
    app.run();
}
