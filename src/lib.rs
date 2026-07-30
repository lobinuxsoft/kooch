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
//! | `camera`   | Authorable camera rigs      | —            |
//! | `world`    | World management            | —            |
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
#[cfg(feature = "camera")]
pub use ome_camera;
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
/// CLI args or falls back to `scenes/default.scene` relative to cwd.
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
///
/// It does carry the asset plugin, in headless form. Asset *identity* is
/// not a rendering concern: a prefab instance in a scene is a reference
/// now, so loading a scene means resolving a guid. Without it the host
/// spawned `missing prefab [...]` for prefabs that were sitting right
/// there. Eager import stays off — decoding every texture for a process
/// that never draws is work with no result.
pub struct RemoteHostPlugins;

impl ome_core::plugin::PluginGroup for RemoteHostPlugins {
    fn build(self) -> ome_core::plugin::PluginGroupBuilder {
        let builder = ome_core::plugin::PluginGroupBuilder::new()
            .add(ome_core::plugin::CorePlugin)
            .add(ome_ecs::EcsPlugin)
            .add(default_asset_plugin().headless());

        // The host is what actually simulates when the editor presses
        // Play, so it needs physics even though it draws nothing.
        #[cfg(feature = "physics")]
        let builder = builder.add(ome_physics::PhysicsPlugin::new());

        // Gravity that points somewhere other than down. Inert until a
        // scene holds a source, so adding it changes nothing on its own.
        #[cfg(all(feature = "physics", feature = "gravity"))]
        let builder = builder.add(ome_gravity::GravityPlugin);

        // Camera rigs run here for the same reason physics does: the host
        // is what simulates, and the editor draws the pose it produced.
        #[cfg(feature = "camera")]
        let builder = builder.add(ome_camera::CameraPlugin);

        // What lets the editor draw the solver's state from over there.
        #[cfg(all(feature = "physics", feature = "remote"))]
        let builder = builder.add(PhysicsRemotePlugin);

        builder.add(SceneBootstrapPlugin::default())
    }
}

pub struct DefaultPlugins;

impl ome_core::plugin::PluginGroup for DefaultPlugins {
    fn build(self) -> ome_core::plugin::PluginGroupBuilder {
        let builder = ome_core::plugin::PluginGroupBuilder::new()
            .add(ome_core::plugin::CorePlugin)
            .add(ome_ecs::EcsPlugin);

        #[cfg(all(feature = "physics", feature = "gravity"))]
        let builder = builder.add(ome_gravity::GravityPlugin);

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

        // Gravity that points somewhere other than down. Inert until a
        // scene holds a source, so adding it changes nothing on its own.
        #[cfg(all(feature = "physics", feature = "gravity"))]
        let builder = builder.add(ome_gravity::GravityPlugin);

        // A camera that follows something is not an optional idea for a
        // 3D game, and the crate is inert until a rig is authored.
        #[cfg(feature = "camera")]
        let builder = builder.add(ome_camera::CameraPlugin);

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
/// `ome_remote` knows about entities and components and deliberately not
/// about physics; `ome_physics` knows about bodies and deliberately not
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
impl ome_core::plugin::Plugin for PhysicsRemotePlugin {
    fn build(&self, app: &mut ome_core::app::App) {
        app.add_system(
            ome_core::stage::Stage::Startup,
            |resources: &mut ome_core::resource::Resources| {
                if !resources.contains::<ome_remote::extensions::RemoteExtensions>() {
                    resources.insert(ome_remote::extensions::RemoteExtensions::default());
                }
                let Some(extensions) =
                    resources.get_mut::<ome_remote::extensions::RemoteExtensions>()
                else {
                    return;
                };
                extensions.register(
                    "physics.debug_lines",
                    Box::new(|resources, payload| {
                        let categories: ome_physics::backend::DebugCategories =
                            debug_categories_from(payload);
                        // Off means off: the walk is per-frame CPU work, and a
                        // request with nothing switched on must not pay for it.
                        if !categories.any() {
                            return Ok(ome_remote::serde_json::json!({ "lines": [] }));
                        }
                        let world = resources
                            .get::<ome_physics::plugin::PhysicsWorld>()
                            .ok_or_else(|| "this host has no physics world".to_owned())?;
                        let mut lines = Vec::new();
                        world.backend().debug_lines(categories, &mut lines);
                        Ok(ome_remote::serde_json::json!({
                            "lines": lines
                                .iter()
                                .map(|line| ome_remote::serde_json::json!({
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
    payload: &ome_remote::serde_json::Value,
) -> ome_physics::backend::DebugCategories {
    let flag = |name: &str| {
        payload
            .get(name)
            .and_then(ome_remote::serde_json::Value::as_bool)
            == Some(true)
    };
    ome_physics::backend::DebugCategories {
        collider_shapes: flag("collider_shapes"),
        contacts: flag("contacts"),
        joints: flag("joints"),
        collider_aabbs: flag("collider_aabbs"),
        body_axes: flag("body_axes"),
    }
}
