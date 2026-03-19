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
//! | `render`   | Clear-color renderer        | `window`     |
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
    pub use ome_render::{ClearColor, RenderPlugin};
    #[cfg(feature = "dynamic")]
    pub use ome_plugin_api::prelude as plugin_api;
}

/// Default set of plugins for a windowed application.
///
/// Includes [`CorePlugin`](ome_core::plugin::CorePlugin),
/// [`EcsPlugin`](ome_ecs::EcsPlugin), and conditionally
/// [`WindowPlugin`](ome_window::WindowPlugin) and
/// [`RenderPlugin`](ome_render::RenderPlugin) based on enabled features.
///
/// # Example
/// ```ignore
/// use oh_my_engine::prelude::*;
/// use oh_my_engine::DefaultPlugins;
///
/// App::new().add_plugins(DefaultPlugins).run();
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
        let builder = builder.add(ome_render::RenderPlugin::default());

        builder
    }
}
