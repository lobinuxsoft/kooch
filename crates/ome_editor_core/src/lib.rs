//! ome_editor_core — Embedded editor overlay for oh_my_engine
//!
//! Provides an egui-based inspector overlay that renders on top of the
//! engine viewport. Includes entity hierarchy, component inspector,
//! and basic spawn/despawn controls.
//!
//! # Usage
//!
//! ```ignore
//! use ome_editor_core::EditorPlugin;
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
pub(crate) mod panels;
pub mod perf;
pub mod play_state;
pub mod project;
pub mod project_state;
pub mod remote_mirror;
pub mod remote_session;
pub(crate) mod queries;
pub(crate) mod state;
pub(crate) mod style;
pub(crate) mod systems;
pub(crate) mod undo;
pub(crate) mod viewport;

use ome_core::app::App;
use ome_core::plugin::Plugin;
use ome_core::stage::Stage;

pub use bootstrap::{run_editor, run_editor_with};
pub use editor_camera::{EditorCamera, EditorCameraController, EditorOnly};
pub use perf::EditorPerfStats;
pub use play_state::PlayState;
pub use project::{EditorConfig, ProjectManifest};
pub use remote_mirror::{MirrorEntity, RemoteMirror};
pub use remote_session::{ConnectionState, RemoteSession, RemoteState};
pub use project_state::ProjectState;
pub use state::EditorOverlay;

/// Plugin that adds the embedded egui editor overlay.
///
/// Requires [`WindowPlugin`](ome_window::WindowPlugin) and
/// [`EcsPlugin`](ome_ecs::EcsPlugin) to be registered first.
///
/// Registers two systems:
/// - **Startup**: initializes egui context, winit integration, and wgpu renderer.
/// - **Render**: draws the overlay UI and presents to the surface.
pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PlayState::new());
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
        // Register the EditorOnly marker as ephemeral *before* the camera
        // is spawned, so the entity is filtered from any save that races
        // the spawn (e.g. play-mode snapshot triggered immediately).
        app.add_system(
            Stage::Startup,
            editor_camera::register_ephemeral_markers_system,
        );
        app.add_system(Stage::Startup, editor_camera::spawn_editor_camera_system);
        // Hand the viewport over to the gameplay camera in play mode.
        app.add_system(
            Stage::PreRender,
            editor_camera::sync_editor_camera_active_system,
        );
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
        // Rebuild the gizmo line batch from current selection. Runs after
        // transform propagation (PostUpdate) so GlobalTransform is fresh.
        app.add_system(Stage::PreRender, gizmos::build_gizmo_batch_system);
        app.add_system(Stage::Render, systems::editor_render_system);
        // Persist any dock-layout changes after the frame finishes.
        // Cheap fast-path: re-serializes and compares before any disk I/O,
        // so steady-state frames produce zero writes.
        app.add_system(Stage::Last, layout::save_layout_system);
    }

    fn name(&self) -> &str {
        "EditorPlugin"
    }
}
