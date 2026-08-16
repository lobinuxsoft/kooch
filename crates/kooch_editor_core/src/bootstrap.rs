//! Reusable editor entry point.
//!
//! Both the standalone `kooch_editor` launcher and a generated project (in
//! its editor build) boot the editor through here, so the plugin set
//! lives in one place. The editor owns the window surface (egui overlay +
//! an offscreen scene target), so it must NOT be combined with
//! `RenderPlugin` / `DefaultPlugins` — those present to the same surface
//! and conflict at `Stage::Render`.

use kooch_core::prelude::*;
use kooch_ecs::EcsPlugin;
use kooch_render::plugin::AssetPlugin;
use kooch_window::WindowPlugin;
use kooch_world::WorldStreamingPlugin;

use crate::EditorPlugin;
use crate::project_state::ProjectState;

/// The window's title: the editor, its version, and the open project.
///
/// One function rather than a `format!` at each of the three sites that
/// set this. The version belongs here because the title bar and the task
/// switcher are where it is legible without opening anything, and
/// because "which editor is running" is the question a stale vendored
/// engine makes someone ask.
pub fn window_title(project: Option<&str>) -> String {
    let version = crate::engine_vendor::editor_engine_version();
    match project {
        Some(name) => format!("{name} — Kóoch {version}"),
        None => format!("Kóoch {version}"),
    }
}

/// Runs the editor with no project plugin — the standalone launcher.
pub fn run_editor() {
    run_editor_with(NoProjectPlugin);
}

/// Runs the editor with a project-supplied plugin (typically the
/// generated `registrations::ProjectRegistrations`) so the project's
/// components + systems are registered and show up in the editor UI.
pub fn run_editor_with<P: Plugin + 'static>(project: P) {
    force_x11_backend_if_needed();
    // With a console buffer beside stdout: the editor has a panel to
    // show it in, and a project opened from the launcher has no terminal
    // attached to read the other one.
    let log_buffer = kooch_core::init_tracing_with_console();

    let mut app = App::new();
    app.insert_resource(log_buffer);
    app.insert_resource(crate::panels::console::ConsoleState::default());
    app.add_plugins(MinimalPlugins);
    app.add_plugin(WindowPlugin {
        title: window_title(None),
        width: 1280,
        height: 720,
    });
    app.add_plugin(EcsPlugin);
    // Asset root resolved to an ABSOLUTE path — the cwd shifts when a
    // project is opened.
    app.add_plugin(AssetPlugin::new().with_root(engine_root().join("assets")));
    app.add_plugin(WorldStreamingPlugin);
    app.add_plugin(EditorPlugin);
    // After EditorPlugin on purpose: both register a raw-event handler in
    // Startup, and the first one registered gets first refusal on a
    // keystroke. egui has to be able to keep what a focused text field
    // typed, or naming an entity would also drive the player (#710).
    app.add_plugin(kooch_input::InputPlugin);
    app.add_plugin(project);
    app.add_system(Stage::Startup, set_engine_root);
    app.run();
}

/// No-op plugin for the launcher's `run_editor()`.
struct NoProjectPlugin;
impl Plugin for NoProjectPlugin {
    fn build(&self, _app: &mut App) {}
}

/// Resolves the engine's asset root at runtime, in order:
///
/// 1. `KOOCH_ENGINE_ROOT` env var if set.
/// 2. The executable's directory, if it has a sibling `assets/`.
/// 3. The first ancestor of the executable containing `assets/` (dev
///    layout, and for a project build resolves to the project itself).
/// 4. Compile-time `CARGO_MANIFEST_DIR` walk (this crate → repo root).
///
/// Panics if none resolve — the editor cannot run without an asset root.
fn engine_root() -> std::path::PathBuf {
    if let Ok(env) = std::env::var("KOOCH_ENGINE_ROOT") {
        let p = std::path::PathBuf::from(env);
        if p.exists() {
            return p;
        }
    }

    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        if dir.join("assets").is_dir() {
            return dir.to_path_buf();
        }
        for ancestor in dir.ancestors().skip(1) {
            if ancestor.join("assets").is_dir() {
                return ancestor.to_path_buf();
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
        "engine_root could not be resolved: set KOOCH_ENGINE_ROOT, ship assets/ next \
         to the executable, or run from a source checkout that contains assets/",
    );
}

/// Startup system that records the engine root on `ProjectState` (so
/// `create_project` can generate valid `Cargo.toml` paths) and honours
/// `KOOCH_EDITOR_AUTO_OPEN` for headless / smoke runs.
fn set_engine_root(resources: &mut Resources) {
    if let Some(ps) = resources.get_mut::<ProjectState>() {
        ps.engine_root = Some(engine_root());
    }

    if let Ok(env_path) = std::env::var("KOOCH_EDITOR_AUTO_OPEN") {
        let path = std::path::PathBuf::from(env_path);
        if path.exists() {
            auto_open_project(resources, &path);
        } else {
            tracing::warn!(
                path = %path.display(),
                "KOOCH_EDITOR_AUTO_OPEN: project path does not exist",
            );
        }
    }
}

/// Opens the project exactly as clicking Open Project does.
///
/// # It used to do something else, and that was the bug
///
/// This called `ProjectState::open_project` directly — the low-level method
/// — which skips both halves of what opening a project means:
/// `SceneSource::RemoteMirror` and `start_remote_session`. So a project
/// opened through this variable got the read-only in-process path: no
/// running project, no gameplay, and a Play button that shelled out to a
/// second window instead of simulating in the viewport.
///
/// Which made this variable a liar. It is documented for smoke runs, and a
/// smoke run has to exercise what an author exercises — otherwise it
/// confirms a world nobody uses. Going through the action means there is
/// one path, and it is the one clicking takes.
fn auto_open_project(resources: &mut Resources, path: &std::path::Path) {
    tracing::info!(path = %path.display(), "KOOCH_EDITOR_AUTO_OPEN: opening project");
    // A throwaway stack: opening a project is not an undoable edit, and
    // the real one belongs to the editor loop that has not started yet.
    let mut undo = crate::undo::UndoStack::new();
    crate::actions::apply_actions(
        resources,
        &[crate::actions::EditorAction::OpenProject(
            path.to_path_buf(),
        )],
        &mut undo,
    );
}

/// Forces winit onto XWayland on Linux by clearing `WAYLAND_DISPLAY`
/// before the event loop is built.
///
/// Workaround for egui's IBus+Wayland IME bug (egui #7485, fixed upstream
/// in #7983 / egui 0.35+): on native Wayland a `TextEdit` accepts only
/// one character before dropping input. XWayland is unaffected. Opt back
/// into native Wayland with `KOOCH_FORCE_WAYLAND=1`.
fn force_x11_backend_if_needed() {
    if !cfg!(target_os = "linux") {
        return;
    }
    if std::env::var_os("KOOCH_FORCE_WAYLAND").is_some() {
        return;
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        // SAFETY: called at the very top of the editor entry point,
        // before any threads are spawned (tracing included) and before
        // the winit event loop reads the env — nothing can race it.
        unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
        eprintln!(
            "kooch_editor: cleared WAYLAND_DISPLAY to force XWayland \
             (egui #7485 IME workaround); set KOOCH_FORCE_WAYLAND=1 to keep native Wayland",
        );
    }
}

#[cfg(test)]
mod tests;
