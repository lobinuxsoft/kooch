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

pub mod icons;
pub mod launch_screen;
pub mod play_state;
pub mod project;
pub mod project_state;
pub(crate) mod actions;
pub(crate) mod menu_bar;
pub(crate) mod panels;
pub(crate) mod queries;
pub(crate) mod state;
pub(crate) mod style;
pub(crate) mod systems;
pub(crate) mod undo;

use ome_core::app::App;
use ome_core::plugin::Plugin;
use ome_core::stage::Stage;

pub use state::EditorOverlay;
pub use play_state::PlayState;
pub use project::{EditorConfig, ProjectManifest};
pub use project_state::ProjectState;

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
        app.add_system(Stage::Startup, systems::editor_startup_system);
        app.add_system(Stage::Render, systems::editor_render_system);
    }

    fn name(&self) -> &str {
        "EditorPlugin"
    }
}
