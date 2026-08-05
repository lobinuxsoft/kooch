//! kooch_editor_core — Embedded editor overlay for kooch
//!
//! Provides an egui-based inspector overlay that renders on top of the
//! engine viewport. Includes entity hierarchy, component inspector,
//! and basic spawn/despawn controls.
//!
//! # Usage
//!
//! ```ignore
//! use kooch_editor_core::EditorPlugin;
//!
//! App::new()
//!     .add_plugin(WindowPlugin::default())
//!     .add_plugin(EcsPlugin)
//!     .add_plugin(EditorPlugin)
//!     .run();
//! ```

pub(crate) mod actions;
pub mod bootstrap;
pub(crate) mod drag_drop;
pub mod editor_camera;
pub(crate) mod gizmos;
pub mod icons;
pub mod launch_screen;
pub(crate) mod layout;
pub(crate) mod menu_bar;
pub(crate) mod numeric;
pub(crate) mod panels;
pub mod perf;
mod picking;
pub mod play_state;
pub mod project;
pub mod project_log;
pub mod project_plugin;
mod project_state;
pub(crate) mod queries;
pub(crate) mod remote_input;
pub mod remote_mirror;
pub mod remote_session;
pub(crate) mod state;
pub(crate) mod style;
pub(crate) mod systems;
pub(crate) mod undo;
pub(crate) mod viewport;
pub(crate) mod viewport_pick;
pub(crate) mod widgets;

use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::stage::Stage;

pub use bootstrap::{run_editor, run_editor_with};
pub use editor_camera::{EditorCamera, EditorCameraController, EditorOnly};
pub use perf::EditorPerfStats;
pub use play_state::PlayState;
pub use project::{EditorConfig, ProjectManifest};
pub use project_state::ProjectState;
pub use remote_mirror::{MirrorEntity, RemoteMirror};
pub use remote_session::{ConnectionState, RemoteSession, RemoteState};
pub use state::EditorOverlay;

/// Plugin that adds the embedded egui editor overlay.
///
/// Requires [`WindowPlugin`](kooch_window::WindowPlugin) and
/// [`EcsPlugin`](kooch_ecs::EcsPlugin) to be registered first.
///
/// Registers two systems:
/// - **Startup**: initializes egui context, winit integration, and wgpu renderer.
/// - **Render**: draws the overlay UI and presents to the surface.
pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        // Physics is authored here but never simulated here: the editor
        // needs PhysicsBody and Collider reflected so they reach the
        // add-component menu and the Inspector, while the project (local
        // Play, or the remote host) owns the solver. Without this the
        // menu offers no body component at all — the registry it reads is
        // the editor's own.
        app.add_plugin(kooch_physics::PhysicsComponentsPlugin);
        // Components only: the editor authors gravity, the project's
        // process applies it.
        app.add_plugin(kooch_gravity::GravityComponentsPlugin);
        // Same split again: the editor authors camera behaviour and never
        // runs it. A vcam driving a camera here would fight the editor's
        // own, which owns the viewport.
        app.add_plugin(kooch_camera::CameraComponentsPlugin);
        // And input: the editor authors which `.inputmap` a scene plays
        // under, the project's process reads it. Without this the menu
        // offers `InputMapSource` — the name reaches it by another route
        // — and adding it fails with "no default value".
        app.add_plugin(kooch_input::actions::InputComponentsPlugin);

        // #656 — the editor sleeps by default and every frame has to
        // earn the next one. The baseline is what the accumulator falls
        // back to after the runner reads it, so a frame that asks for
        // nothing is a frame that stops the loop.
        app.insert_resource(kooch_core::frame_pacing::FrameRequest::new(
            kooch_core::frame_pacing::FramePace::Wait,
        ));
        app.insert_resource(PlayState::new());
        // Remote mode starts inert: no session means the editor drives
        // its own ECS exactly as before. "Open Remote" fills it in.
        app.insert_resource(remote_session::RemoteState::new());
        app.insert_resource(systems::RemoteSyncState::default());
        app.insert_resource(project_state::ProjectState::new());
        app.insert_resource(undo::UndoStack::new());
        // #463 perf HUD — populated incrementally by per-metric
        // systems (frame timer, sysinfo poller, GPU timestamp
        // readback, render-side counters). Inserted at zero so the
        // toolbar can read it on the very first frame without any
        // metric system having run yet.
        app.insert_resource(perf::EditorPerfStats::default());
        app.insert_resource(perf::PerfTimingState::default());
        app.insert_resource(perf::SysMetricsState::default());
        app.insert_resource(editor_camera::EditorCameraController::default());
        app.insert_resource(layout::LayoutPersistence::default());
        // The engine's own frame reporting is for a game, which has no
        // other way to say how fast it runs (#698). The editor has the
        // perf HUD, so a line per second in the Console is noise on top of
        // a number already on screen — and the Console keeps every line it
        // is given.
        //
        // Silenced in `Startup` rather than by overwriting the resource
        // here, because `CorePlugin` builds it from the environment and
        // which plugin's `build` runs last is not a thing to depend on.
        // The variable stays set in the process environment, so a game
        // launched by Play still inherits it and still reports.
        app.add_system(
            Stage::Startup,
            |resources: &mut kooch_core::resource::Resources| {
                if let Some(metrics) =
                    resources.get_mut::<kooch_core::frame_metrics::FrameMetrics>()
                {
                    metrics.report = kooch_core::frame_metrics::MetricsReport::Silent;
                }
            },
        );
        app.add_system(Stage::Startup, systems::editor_startup_system);
        // Loads the saved dock layout from disk (if any) and replaces the
        // overlay's default. Must run AFTER editor_startup_system so the
        // overlay exists.
        app.add_system(Stage::Startup, layout::load_layout_system);
        // React to project open / close: rescan the project's assets/
        // tree into the AssetDatabase + eager-import. PreUpdate runs
        // before the inspector renders, so the picker sees [project]
        // entries the same frame the user opens a project.
        app.add_system(Stage::PreUpdate, systems::scan_project_assets_system);
        app.add_system(Stage::PreUpdate, systems::ensure_main_exists_system);
        // Remote mode: advance the handshake and pull the project's
        // world into the local mirror. PreUpdate so the panels and the
        // viewport see a snapshot that is at most one frame stale.
        app.add_system(Stage::PreUpdate, systems::remote_sync_system);
        // After the pull, so a snapshot describes the frame the editor is
        // about to draw rather than the one it just finished.
        app.add_system(Stage::PreUpdate, remote_input::send_input_to_host);
        // Register the EditorOnly marker as ephemeral *before* the camera
        // is spawned, so the entity is filtered from any save that races
        // the spawn (e.g. play-mode snapshot triggered immediately).
        app.add_system(
            Stage::Startup,
            editor_camera::register_ephemeral_markers_system,
        );
        app.add_system(Stage::Startup, editor_camera::spawn_editor_camera_system);

        // #463 perf HUD — sample wall-clock delta between successive
        // editor render invocations and update FPS instant/avg.
        // Runs in PreRender so the timestamp it captures matches the
        // frame the Render stage is about to start.
        app.add_system(Stage::PreRender, perf::frame_timer_system);
        // #463.3 — refresh CPU% / RAM RSS at most twice per second.
        // PreRender keeps the metric and the FPS timer phase-locked
        // so both update before the toolbar reads them.
        app.add_system(Stage::PreRender, perf::sys_metrics_system);
        // Register built-in visualizers (Transform, ...) into the
        // VisualizerRegistry. Must run BEFORE the first gizmo batch build.
        app.add_system(Stage::Startup, gizmos::register_builtin_visualizers_system);
        // Which gizmo groups draw, restored from disk. After the
        // visualizers are registered so the panel has something to list.
        app.add_system(Stage::Startup, gizmos::load_visibility_system);
        // Rebuild the gizmo line batch from current selection. Runs after
        // transform propagation (PostUpdate) so GlobalTransform is fresh.
        app.add_system(Stage::PreRender, gizmos::build_gizmo_batch_system);
        app.add_system(Stage::Render, systems::editor_render_system);
        // Persist any dock-layout changes after the frame finishes.
        // Cheap fast-path: re-serializes and compares before any disk I/O,
        // so steady-state frames produce zero writes.
        app.add_system(Stage::Last, layout::save_layout_system);
        // Same cheap fast-path as the layout: re-serialize, compare, and
        // only touch disk when a choice actually changed.
        app.add_system(Stage::Last, gizmos::save_visibility_system);
    }

    fn name(&self) -> &str {
        "EditorPlugin"
    }
}

#[cfg(test)]
mod engine_component_registration_tests {
    /// 🔴 Every `*ComponentsPlugin` in the workspace is added by the
    /// editor.
    ///
    /// The editor keeps its **own** `ComponentRegistry`. A component the
    /// project registers is invisible here, so authoring one requires the
    /// matching components-plugin in `EditorPlugin::build` — and the list
    /// is written by hand, which is exactly as reliable as it sounds:
    /// this has now been the fifth omission (#722 was the third,
    /// `InputMapSource` the fifth), and each one surfaces as a component
    /// the menu offers and then refuses with "no default value".
    ///
    /// Scanning the source rather than a registry because the failure is
    /// a plugin that was never *added* — a runtime check could only see
    /// what was added, which is the set that is already correct.
    #[test]
    fn every_components_plugin_is_added_by_the_editor() {
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crates/ dir");
        let lib = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"),
        )
        .expect("read the editor's own lib.rs");

        let mut found = Vec::new();
        collect_components_plugins(crates, &mut found);
        assert!(
            found.len() >= 4,
            "the scan found {} plugins, so it is not scanning: {found:?}",
            found.len()
        );

        for name in &found {
            assert!(
                lib.contains(name.as_str()),
                "{name} exists but `EditorPlugin::build` never adds it, so its \
                 components cannot be authored — the add-component menu will \
                 offer them and fail with \"no default value\"",
            );
        }
    }

    /// Every `pub struct <X>ComponentsPlugin` under `dir`.
    fn collect_components_plugins(dir: &std::path::Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                collect_components_plugins(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                for line in text.lines() {
                    let line = line.trim();
                    let Some(rest) = line.strip_prefix("pub struct ") else {
                        continue;
                    };
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if name.ends_with("ComponentsPlugin") && !out.contains(&name) {
                        out.push(name);
                    }
                }
            }
        }
    }
}
