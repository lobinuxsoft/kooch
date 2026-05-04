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
#[cfg(feature = "window")]
pub use ome_window;
#[cfg(feature = "render")]
pub use ome_render;
#[cfg(feature = "gizmos")]
pub use ome_gizmos;
#[cfg(feature = "input")]
pub use ome_input;
#[cfg(feature = "audio")]
pub use ome_audio;
#[cfg(feature = "sdf")]
pub use ome_sdf;
#[cfg(feature = "lighting")]
pub use ome_lighting;
#[cfg(feature = "physics")]
pub use ome_physics;
#[cfg(feature = "gravity")]
pub use ome_gravity;
#[cfg(feature = "world")]
pub use ome_world;
#[cfg(feature = "scripting")]
pub use ome_scripting;
#[cfg(feature = "editor")]
pub use ome_editor_core;

pub use scene_bootstrap::SceneBootstrapPlugin;

/// Convenient re-exports for common usage.
///
/// ```ignore
/// use oh_my_engine::prelude::*;
/// ```
pub mod prelude {
    pub use ome_core::prelude::*;
    pub use ome_ecs::{EcsPlugin, Entity, EntityAllocator};

    #[cfg(feature = "window")]
    pub use ome_window::{WindowCloseRequested, WindowHandle, WindowPlugin, WindowResized};
    #[cfg(feature = "render")]
    pub use ome_render::RenderPlugin;
    #[cfg(feature = "dynamic")]
    pub use ome_plugin_api::prelude as plugin_api;

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
            .add(ome_render::plugin::AssetPlugin::default())
            .add(ome_render::RenderPlugin);

        #[cfg(feature = "world")]
        let builder = builder.add(ome_world::WorldStreamingPlugin);

        builder.add(SceneBootstrapPlugin::default())
    }
}
