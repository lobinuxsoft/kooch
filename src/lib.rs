//! Kooch — GPU-driven game engine.
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
//! | `camera`   | Authorable camera rigs      | —            |
//! | `world`    | World management            | —            |
//! | `editor`   | Editor UI                   | —            |
//!
//! Default features: `window`, `render`.

mod scene_bootstrap;

// Always present
pub use kooch_core;
pub use kooch_ecs;

// Dynamic plugin API (optional)
#[cfg(feature = "dynamic")]
pub use kooch_plugin_api;

// Conditional re-exports
#[cfg(feature = "audio")]
pub use kooch_audio;
#[cfg(feature = "camera")]
pub use kooch_camera;
#[cfg(feature = "editor")]
pub use kooch_editor_core;
#[cfg(feature = "gizmos")]
pub use kooch_gizmos;
#[cfg(feature = "gravity")]
pub use kooch_gravity;
#[cfg(feature = "input")]
pub use kooch_input;
#[cfg(feature = "lighting")]
pub use kooch_lighting;
#[cfg(feature = "physics")]
pub use kooch_physics;
#[cfg(feature = "remote")]
pub use kooch_remote;
#[cfg(feature = "render")]
pub use kooch_render;
#[cfg(feature = "window")]
pub use kooch_window;
#[cfg(feature = "world")]
pub use kooch_world;

pub use scene_bootstrap::SceneBootstrapPlugin;

/// Convenient re-exports for common usage.
///
/// ```ignore
/// use kooch::prelude::*;
/// ```
pub mod prelude {
    pub use kooch_core::prelude::*;
    pub use kooch_ecs::{EcsPlugin, Entity, EntityAllocator};

    #[cfg(feature = "input")]
    pub use kooch_input::{
        InputBackend, InputPlugin, KeyCode, MouseButton,
        backend::{GamepadAxis, GamepadButton, GamepadId},
    };
    #[cfg(feature = "physics")]
    pub use kooch_physics::{Collider, PhysicsPlugin, RigidBody};
    #[cfg(feature = "dynamic")]
    pub use kooch_plugin_api::prelude as plugin_api;
    #[cfg(feature = "remote")]
    pub use kooch_remote::RemotePlugin;
    #[cfg(feature = "render")]
    pub use kooch_render::RenderPlugin;
    #[cfg(feature = "window")]
    pub use kooch_window::{WindowCloseRequested, WindowHandle, WindowPlugin, WindowResized};

    #[cfg(feature = "remote")]
    pub use crate::RemoteHostPlugins;
    pub use crate::{DefaultPlugins, SceneBootstrapPlugin};
}

/// Default set of plugins for a windowed game application.
///
/// Includes [`CorePlugin`](kooch_core::plugin::CorePlugin),
/// [`EcsPlugin`](kooch_ecs::EcsPlugin), [`SceneBootstrapPlugin`], and
/// conditionally [`WindowPlugin`](kooch_window::WindowPlugin) and
/// [`RenderPlugin`](kooch_render::RenderPlugin) based on enabled features.
///
/// `SceneBootstrapPlugin` resolves the initial scene from `--scene <path>`
/// CLI args or falls back to `scenes/default.scene` relative to cwd.
///
/// # Example
/// ```ignore
/// use kooch::prelude::*;
///
/// fn main() {
///     let mut app = App::new();
///     app.add_plugins(DefaultPlugins);
///     app.run();
/// }
/// ```
/// Builds the engine-side `AssetPlugin` honoring the `KOOCH_ENGINE_ROOT`
/// and `KOOCH_PROJECT_ROOT` env vars the editor's launcher injects when
/// it spawns a game binary in Play mode. With both set, the plugin's
/// primary `asset_root` is `<engine>/assets` (so engine GUIDs resolve)
/// and `<project>/assets` rides as a secondary scan target (so project-
/// authored assets are visible too).
///
/// Without the env vars (game binary launched outside the editor) the
/// plugin falls back to `<exe_dir>/assets` if it exists, otherwise
/// the historical `assets/` working-directory default.
#[cfg(feature = "render")]
fn default_asset_plugin() -> kooch_render::plugin::AssetPlugin {
    use std::path::PathBuf;

    let engine_root = std::env::var_os("KOOCH_ENGINE_ROOT").map(PathBuf::from);
    let project_root = std::env::var_os("KOOCH_PROJECT_ROOT").map(PathBuf::from);

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

    let mut plugin = kooch_render::plugin::AssetPlugin::new().with_root(primary);
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
///
/// It does carry the asset plugin, in headless form. Asset *identity* is
/// not a rendering concern: a prefab instance in a scene is a reference
/// now, so loading a scene means resolving a guid. Without it the host
/// spawned `missing prefab [...]` for prefabs that were sitting right
/// there. Eager import stays off — decoding every texture for a process
/// that never draws is work with no result.
pub struct RemoteHostPlugins;

impl kooch_core::plugin::PluginGroup for RemoteHostPlugins {
    fn build(self) -> kooch_core::plugin::PluginGroupBuilder {
        let builder = kooch_core::plugin::PluginGroupBuilder::new()
            .add(kooch_core::plugin::CorePlugin)
            .add(kooch_ecs::EcsPlugin)
            .add(default_asset_plugin().headless());

        // The host is what actually simulates when the editor presses
        // Play, so it needs physics even though it draws nothing.
        #[cfg(feature = "physics")]
        let builder = builder.add(kooch_physics::PhysicsPlugin::new());

        // Gravity that points somewhere other than down. Inert until a
        // scene holds a source, so adding it changes nothing on its own.
        #[cfg(all(feature = "physics", feature = "gravity"))]
        let builder = builder.add(kooch_gravity::GravityPlugin);

        // Camera rigs run here for the same reason physics does: the host
        // is what simulates, and the editor draws the pose it produced.
        #[cfg(feature = "camera")]
        let builder = builder.add(kooch_camera::CameraPlugin);

        // What lets the editor draw the solver's state from over there.
        #[cfg(all(feature = "physics", feature = "remote"))]
        let builder = builder.add(PhysicsRemotePlugin);

        builder.add(SceneBootstrapPlugin::default())
    }
}

pub struct DefaultPlugins;

impl kooch_core::plugin::PluginGroup for DefaultPlugins {
    fn build(self) -> kooch_core::plugin::PluginGroupBuilder {
        let builder = kooch_core::plugin::PluginGroupBuilder::new()
            .add(kooch_core::plugin::CorePlugin)
            .add(kooch_ecs::EcsPlugin);

        #[cfg(all(feature = "physics", feature = "gravity"))]
        let builder = builder.add(kooch_gravity::GravityPlugin);

        #[cfg(feature = "window")]
        let builder = builder.add(kooch_window::WindowPlugin::default());

        // Keyboard, mouse and gamepad. Needs the window: its events are
        // what feed the keyboard, so a headless app gets nothing from it
        // and the host in `RemoteHostPlugins` deliberately has neither.
        #[cfg(all(feature = "window", feature = "input"))]
        let builder = builder.add(kooch_input::InputPlugin);

        #[cfg(feature = "render")]
        let builder = builder
            .add(default_asset_plugin())
            .add(kooch_render::RenderPlugin);

        #[cfg(feature = "world")]
        let builder = builder.add(kooch_world::WorldStreamingPlugin);

        #[cfg(feature = "physics")]
        let builder = builder.add(kooch_physics::PhysicsPlugin::new());

        // Gravity that points somewhere other than down. Inert until a
        // scene holds a source, so adding it changes nothing on its own.
        #[cfg(all(feature = "physics", feature = "gravity"))]
        let builder = builder.add(kooch_gravity::GravityPlugin);

        // A camera that follows something is not an optional idea for a
        // 3D game, and the crate is inert until a rig is authored.
        #[cfg(feature = "camera")]
        let builder = builder.add(kooch_camera::CameraPlugin);

        // What lets the editor draw the solver's state from over there.
        #[cfg(all(feature = "physics", feature = "remote"))]
        let builder = builder.add(PhysicsRemotePlugin);

        builder.add(SceneBootstrapPlugin::default())
    }
}

/// Lets the editor ask the host for the solver's own account of itself.
///
/// # Why this lives in the facade
///
/// `kooch_remote` knows about entities and components and deliberately not
/// about physics; `kooch_physics` knows about bodies and deliberately not
/// about wires. Neither should learn the other. This crate already depends
/// on both, so it is the one place they can meet — the extension registry
/// exists exactly so this can be a plugin rather than a dependency.
///
/// Serves `physics.debug_lines`: takes the categories to draw and returns
/// world-space segments. The editor's overlay reads `PhysicsWorld` when it
/// has one and asks over the wire when it does not, which in the editor is
/// always (#634).
#[cfg(all(feature = "physics", feature = "remote"))]
pub struct PhysicsRemotePlugin;

#[cfg(all(feature = "physics", feature = "remote"))]
impl kooch_core::plugin::Plugin for PhysicsRemotePlugin {
    fn build(&self, app: &mut kooch_core::app::App) {
        app.add_system(
            kooch_core::stage::Stage::Startup,
            |resources: &mut kooch_core::resource::Resources| {
                if !resources.contains::<kooch_remote::extensions::RemoteExtensions>() {
                    resources.insert(kooch_remote::extensions::RemoteExtensions::default());
                }
                let Some(extensions) =
                    resources.get_mut::<kooch_remote::extensions::RemoteExtensions>()
                else {
                    return;
                };
                extensions.register(
                    "physics.debug_lines",
                    Box::new(|resources, payload| {
                        let categories: kooch_physics::backend::DebugCategories =
                            debug_categories_from(payload);
                        // Off means off: the walk is per-frame CPU work, and a
                        // request with nothing switched on must not pay for it.
                        if !categories.any() {
                            return Ok(kooch_remote::serde_json::json!({ "lines": [] }));
                        }
                        let world = resources
                            .get::<kooch_physics::plugin::PhysicsWorld>()
                            .ok_or_else(|| "this host has no physics world".to_owned())?;
                        let mut lines = Vec::new();
                        world.backend().debug_lines(categories, &mut lines);
                        Ok(kooch_remote::serde_json::json!({
                            "lines": lines
                                .iter()
                                .map(|line| kooch_remote::serde_json::json!({
                                    "start": line.start.to_array(),
                                    "end": line.end.to_array(),
                                    "color": line.color.to_array(),
                                }))
                                .collect::<Vec<_>>(),
                        }))
                    }),
                );
            },
        );
    }

    fn name(&self) -> &str {
        "PhysicsRemotePlugin"
    }
}

/// Reads the five switches out of the request payload.
///
/// A missing switch is off rather than an error: a client asking for
/// contacts alone should not have to spell out the four it does not want,
/// and a newer editor asking for a category this host has never heard of
/// should get the rest instead of a failure.
#[cfg(all(feature = "physics", feature = "remote"))]
fn debug_categories_from(
    payload: &kooch_remote::serde_json::Value,
) -> kooch_physics::backend::DebugCategories {
    let flag = |name: &str| {
        payload
            .get(name)
            .and_then(kooch_remote::serde_json::Value::as_bool)
            == Some(true)
    };
    kooch_physics::backend::DebugCategories {
        collider_shapes: flag("collider_shapes"),
        contacts: flag("contacts"),
        joints: flag("joints"),
        collider_aabbs: flag("collider_aabbs"),
        body_axes: flag("body_axes"),
    }
}
