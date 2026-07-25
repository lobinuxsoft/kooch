//! OhMyEngine — GPU-driven game engine.
//!
//! Facade crate that re-exports all engine modules.
//! Use Cargo features to control which modules are compiled.
//!
//! # Features
//!
//! | Feature    | Description                | Dependencies |
//! |------------|----------------------------|--------------|
//! | `window`   | Windowing via winit         | —            |
//! | `render`   | Full game render pipeline   | `window`     |
//! | `input`    | Gamepad/keyboard input      | `window`     |
//! | `audio`    | Audio playback via kira     | —            |
//! | `sdf`      | Signed distance fields      | —            |
//! | `lighting` | Lighting system             | —            |
//! | `physics`  | Physics simulation          | —            |
//! | `gravity`  | Gravity system              | —            |
//! | `world`    | World management            | —            |
//! | `scripting`| Scripting via rhai          | —            |
//! | `editor`   | Editor UI                   | —            |
//!
//! Default features: `window`, `render`.

mod scene_bootstrap;

// Always present
pub use ome_core;
pub use ome_ecs;

// Dynamic plugin API (optional)
#[cfg(feature = "dynamic")]
pub use ome_plugin_api;

// Conditional re-exports
#[cfg(feature = "audio")]
pub use ome_audio;
#[cfg(feature = "editor")]
pub use ome_editor_core;
#[cfg(feature = "gizmos")]
pub use ome_gizmos;
#[cfg(feature = "gravity")]
pub use ome_gravity;
#[cfg(feature = "input")]
pub use ome_input;
#[cfg(feature = "lighting")]
pub use ome_lighting;
#[cfg(feature = "physics")]
pub use ome_physics;
#[cfg(feature = "remote")]
pub use ome_remote;
#[cfg(feature = "render")]
pub use ome_render;
#[cfg(feature = "scripting")]
pub use ome_scripting;
#[cfg(feature = "window")]
pub use ome_window;
#[cfg(feature = "world")]
pub use ome_world;

pub use scene_bootstrap::SceneBootstrapPlugin;

/// Convenient re-exports for common usage.
///
/// ```ignore
/// use oh_my_engine::prelude::*;
/// ```
pub mod prelude {
    pub use ome_core::prelude::*;
    pub use ome_ecs::{EcsPlugin, Entity, EntityAllocator};

    #[cfg(feature = "physics")]
    pub use ome_physics::{Collider, PhysicsPlugin, RigidBody};
    #[cfg(feature = "dynamic")]
    pub use ome_plugin_api::prelude as plugin_api;
    #[cfg(feature = "remote")]
    pub use ome_remote::RemotePlugin;
    #[cfg(feature = "render")]
    pub use ome_render::RenderPlugin;
    #[cfg(feature = "window")]
    pub use ome_window::{WindowCloseRequested, WindowHandle, WindowPlugin, WindowResized};

    #[cfg(feature = "remote")]
    pub use crate::RemoteHostPlugins;
    pub use crate::{DefaultPlugins, SceneBootstrapPlugin};
}

/// Default set of plugins for a windowed game application.
///
/// Includes [`CorePlugin`](ome_core::plugin::CorePlugin),
/// [`EcsPlugin`](ome_ecs::EcsPlugin), [`SceneBootstrapPlugin`], and
/// conditionally [`WindowPlugin`](ome_window::WindowPlugin) and
/// [`RenderPlugin`](ome_render::RenderPlugin) based on enabled features.
///
/// `SceneBootstrapPlugin` resolves the initial scene from `--scene <path>`
/// CLI args or falls back to `scenes/default.ome_scene` relative to cwd.
///
/// # Example
/// ```ignore
/// use oh_my_engine::prelude::*;
///
/// fn main() {
///     let mut app = App::new();
///     app.add_plugins(DefaultPlugins);
///     app.run();
/// }
/// ```
/// Builds the engine-side `AssetPlugin` honoring the `OME_ENGINE_ROOT`
/// and `OME_PROJECT_ROOT` env vars the editor's launcher injects when
/// it spawns a game binary in Play mode. With both set, the plugin's
/// primary `asset_root` is `<engine>/assets` (so engine GUIDs resolve)
/// and `<project>/assets` rides as a secondary scan target (so project-
/// authored assets are visible too).
///
/// Without the env vars (game binary launched outside the editor) the
/// plugin falls back to `<exe_dir>/assets` if it exists, otherwise
/// the historical `assets/` working-directory default.
#[cfg(feature = "render")]
fn default_asset_plugin() -> ome_render::plugin::AssetPlugin {
    use std::path::PathBuf;

    let engine_root = std::env::var_os("OME_ENGINE_ROOT").map(PathBuf::from);
    let project_root = std::env::var_os("OME_PROJECT_ROOT").map(PathBuf::from);

    let primary = engine_root
        .as_ref()
        .map(|p| p.join("assets"))
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|e| e.parent().map(|p| p.join("assets")))
                .filter(|p| p.exists())
        })
        .unwrap_or_else(|| PathBuf::from("assets"));

    let mut plugin = ome_render::plugin::AssetPlugin::new().with_root(primary);
    if let Some(project) = project_root {
        let project_assets = project.join("assets");
        if project_assets.exists() {
            plugin = plugin.with_extra_root(project_assets);
        }
    }
    plugin
}

/// Plugin set for a project running as a **remote authoring host**
/// (`cargo run -- --remote`).
///
/// Everything [`DefaultPlugins`] has minus the window and the renderer:
/// the project owns the ECS and answers the editor over HTTP, while the
/// editor draws the world in its own viewport. Opening a second window
/// here would show the same scene twice and steal focus from the editor
/// — the project is a headless host, not a game.
pub struct RemoteHostPlugins;

impl ome_core::plugin::PluginGroup for RemoteHostPlugins {
    fn build(self) -> ome_core::plugin::PluginGroupBuilder {
        let builder = ome_core::plugin::PluginGroupBuilder::new()
            .add(ome_core::plugin::CorePlugin)
            .add(ome_ecs::EcsPlugin);

        // The host is what actually simulates when the editor presses
        // Play, so it needs physics even though it draws nothing.
        #[cfg(feature = "physics")]
        let builder = builder.add(ome_physics::PhysicsPlugin::new());

        builder.add(SceneBootstrapPlugin::default())
    }
}

pub struct DefaultPlugins;

impl ome_core::plugin::PluginGroup for DefaultPlugins {
    fn build(self) -> ome_core::plugin::PluginGroupBuilder {
        let builder = ome_core::plugin::PluginGroupBuilder::new()
            .add(ome_core::plugin::CorePlugin)
            .add(ome_ecs::EcsPlugin);

        #[cfg(feature = "window")]
        let builder = builder.add(ome_window::WindowPlugin::default());

        #[cfg(feature = "render")]
        let builder = builder
            .add(default_asset_plugin())
            .add(ome_render::RenderPlugin);

        #[cfg(feature = "world")]
        let builder = builder.add(ome_world::WorldStreamingPlugin);

        #[cfg(feature = "physics")]
        let builder = builder.add(ome_physics::PhysicsPlugin::new());

        builder.add(SceneBootstrapPlugin::default())
    }
}
