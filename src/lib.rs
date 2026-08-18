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

/// The engine's licence, verbatim.
///
/// 🔴 **Compiled into every binary that links the engine**, which is
/// what makes it non-optional: a game links Kóoch as an `rlib`, so this
/// string is inside the shipped executable whether or not anyone
/// remembered to copy a file next to it. Removing it means not using
/// the engine.
///
/// The engine's source is protected by this licence rather than by
/// being hidden — Rust has no stable ABI, so a project compiles the
/// engine from source (see #754). Unreal distributes their C++ the same
/// way, on the same basis.
pub const LICENSE: &str = include_str!("../LICENSE.md");

// Named `profiler` and not `profiling` on purpose: a module of that name
// in the crate root shadows the `profiling` facade crate for every path
// written in this file.
#[cfg(feature = "profiling")]
pub mod profiler;
mod scene_bootstrap;
pub mod shipped;

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

/// What a game needs, in one import.
///
/// ```ignore
/// use kooch::prelude::*;
/// ```
///
/// # What belongs here
///
/// One rule: **a game names it**. Components you attach, what a system
/// touches, what you register at startup. Engine machinery does not
/// qualify — `ArchetypeRegistry`, `BodySpec`, `RenderGraph`, `BodyHandle`
/// and friends stay reachable at their full paths.
///
/// The reason for a rule rather than a list is that both failure modes
/// are real. Too narrow and a finished feature is indistinguishable from
/// one that was never built: `Query`, `Transform` and `gravity_at` all
/// existed for months and nothing outside the engine ever named them.
/// Too wide and the prelude stops answering *"what am I supposed to
/// use?"*, collides with names a game wants for itself — `Collider`,
/// `Name`, `Transform` are exactly what a project calls its own types —
/// and quietly promises API stability for internals.
///
/// ⚠️ A prelude entry makes something **findable**, not **used**. A
/// capability nothing calls is still disconnected after it lands here;
/// see `docs/CAPABILITIES.md`.
pub mod prelude {
    pub use kooch_core::prelude::*;

    // The maths types every component is made of. Re-exported rather than
    // left to the project, because the lock holds five versions of glam
    // (0.29 through 0.33, pulled in by rapier, egui and others) and a
    // project adding its own dependency can silently pick a different one
    // than the engine it is talking to (#657).
    pub use glam;
    pub use glam::{Mat3, Mat4, Quat, Vec2, Vec3, Vec4};

    // Logging, for the same reason as glam: a game that wants to say
    // something to the editor's Console had to add a `tracing` dependency
    // of its own and match the engine's version. It is the engine's
    // console — the crate that owns it should hand over the way to write
    // to it.
    pub use tracing;
    pub use tracing::{debug, error, info, warn};

    // The type of every asset reference. A component that points at a
    // mesh, a prefab or an action has a `Guid` field, so a game names it
    // the first time it writes one — and had to reach for
    // `kooch::kooch_core::Guid` to do it.
    pub use kooch_core::Guid;

    // What a game touches on day one. These were reachable all along at
    // `kooch::kooch_ecs::…`, which is to say: only if you already knew
    // they existed. See `docs/CAPABILITIES.md` — the prelude is the
    // discovery surface, and anything missing from it reads, from
    // outside, exactly like a feature that was never built.
    pub use kooch_ecs::{
        Children, Commands, Component, ComponentId, ComponentRegistry, ComponentStorage, EcsPlugin,
        Entity, EntityAllocator, GlobalTransform, MeshRenderer, Name, OrthographicCamera, Parent,
        PerspectiveCamera, Reflect, SceneManager, Transform,
    };
    // The rest of what a scene is made of: what lights it, what the sky
    // is, and the override that pins an entity's level of detail.
    pub use kooch_ecs::{DirectionalLight, LodForceLevel, PointLight, SkyRenderer, SpotLight};
    // Iterating entities by the components they carry, instead of asking
    // the registry for one storage at a time and joining by hand — which
    // is what three engine crates still do, 37 times over.
    pub use kooch_ecs::{Query, With, Without};

    #[cfg(feature = "input")]
    pub use kooch_input::{
        InputBackend, InputPlugin, KeyCode, MouseButton,
        backend::{GamepadAxis, GamepadButton, GamepadId},
    };
    // Actions as data. A game points a component at a `.inputaction` and
    // reads it through `LoadedActions`; nothing in gameplay mentions a
    // key, and nothing names an action.
    #[cfg(feature = "input")]
    pub use kooch_input::actions::{
        Action, ActionId, ActionValue, ActionsPlugin, Binding, Composite, ControlPath, ControlType,
        DeviceClass, InputAction, InputComponentsPlugin, LoadedActions, PartName, Processor,
        VectorMode,
    };
    // `PhysicsWorld` is how a system pushes anything, and `SolverBody`
    // is what addresses a body — both were reachable only by full path,
    // which is why gameplay reached past them for `backend_mut()`.
    #[cfg(feature = "physics")]
    pub use kooch_physics::{
        Collider, Joint, PhysicsBody, PhysicsPlugin, PhysicsWorld, RayHit, SolverBody,
    };

    // `gravity_at` answers "which way is down here", and is the only
    // honest way to ask it: a controller that works it out differently
    // from the solver ends up disagreeing about where the floor is.
    #[cfg(feature = "gravity")]
    pub use kooch_gravity::{
        AreaGravity, BoxGravity, GlobalGravity, GravityPlugin, PointGravity, gravity_at, gravity_up,
    };

    // The mode constants come along: without them `VirtualCamera` cannot
    // be configured from code at all, and `UP_GRAVITY` is what makes a
    // camera work while orbiting a planet.
    #[cfg(feature = "camera")]
    pub use kooch_camera::{
        CameraPlugin, CameraTarget, FOLLOW_GLUED, FOLLOW_NONE, FOLLOW_SIMPLE, FOLLOW_THIRD_PERSON,
        LOOK_AT_MIMIC, LOOK_AT_NONE, LOOK_AT_SIMPLE, UP_GRAVITY, UP_TARGET, UP_WORLD,
        VirtualCamera,
    };

    // Playing a sound is gameplay; the mixer behind it is not.
    #[cfg(feature = "audio")]
    pub use kooch_audio::{AudioBackend, InstanceHandle, PlayParams, SoundHandle};

    // Debug drawing from a game system — a ray you want to see, a radius
    // you are tuning.
    #[cfg(feature = "gizmos")]
    pub use kooch_gizmos::Gizmos;

    #[cfg(feature = "dynamic")]
    pub use kooch_plugin_api::prelude as plugin_api;
    #[cfg(feature = "remote")]
    pub use kooch_remote::RemotePlugin;
    #[cfg(feature = "render")]
    pub use kooch_render::RenderPlugin;
    #[cfg(feature = "window")]
    pub use kooch_window::{WindowCloseRequested, WindowHandle, WindowPlugin, WindowResized};
    // What the streaming system follows. The chunk machinery around it
    // stays internal.
    #[cfg(feature = "world")]
    pub use kooch_world::StreamingFocus;

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
    use std::path::{Path, PathBuf};

    let engine_root = std::env::var_os("KOOCH_ENGINE_ROOT").map(PathBuf::from);
    let project_root = std::env::var_os("KOOCH_PROJECT_ROOT").map(PathBuf::from);

    // 🔴 A shipped game's assets live in a pack, so `<exe>/assets` is
    // the right root even though no such directory exists — the
    // `.exists()` filter below would reject it and fall through to the
    // working directory, which for a double-clicked game is the user's
    // home. Same failure the boot scene had.
    let shipped = crate::shipped::shipped_pack();
    let beside_exe = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.join("assets")));

    let primary = engine_root
        .as_ref()
        .map(|p| p.join("assets"))
        .or_else(|| match shipped.is_some() {
            true => beside_exe.clone(),
            false => beside_exe.clone().filter(|p| p.exists()),
        })
        .unwrap_or_else(|| PathBuf::from("assets"));

    // No loader list here on purpose. Every asset type declares itself
    // beside its own definition with `kooch_core::register_asset!`, and
    // `AssetPlugin` installs whatever is linked in. A list in the facade
    // meant the editor kept a second copy of it, and the two drifted.
    let mut plugin = kooch_render::plugin::AssetPlugin::new().with_root(primary);
    if let Some((pack, key)) = shipped {
        tracing::info!(target: "kooch::shipped", path = %pack.display(), "reading assets from the shipped pack");
        // 🔴 Mounted over the game folder, not over `assets/`. The pack
        // holds `assets/…` *and* `scenes/…`, because a scene is the
        // structure of the whole game and shipping it in plain RON beside
        // an encrypted pack protects the textures and publishes the
        // design. One mount covers both.
        let root = pack.parent().map(Path::to_path_buf).unwrap_or_default();
        plugin = plugin.with_pack_over(root, pack, key);
    }
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

        // Input the editor captured on our behalf. Without it a headless
        // host is a process no key can reach (#710).
        #[cfg(all(feature = "input", feature = "remote"))]
        let builder = builder.add(InputRemotePlugin);

        builder.add(SceneBootstrapPlugin::default())
    }
}

pub struct DefaultPlugins;

impl kooch_core::plugin::PluginGroup for DefaultPlugins {
    fn build(self) -> kooch_core::plugin::PluginGroupBuilder {
        let builder = kooch_core::plugin::PluginGroupBuilder::new()
            .add(kooch_core::plugin::CorePlugin)
            .add(kooch_ecs::EcsPlugin);

        // First, so the socket is already listening while the asset
        // loaders do the slowest work of the run — and so the author of
        // the game never edits a line to be able to profile it. Absent
        // from a build that did not ask for the feature.
        #[cfg(feature = "profiling")]
        let builder = builder.add(crate::profiler::ProfilingPlugin::default());

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

/// Lets the editor hand this host the input it captured.
///
/// # Why the host cannot read input itself
///
/// `RemoteHostPlugins` has no window, on purpose: the editor draws this
/// world in its own viewport. But keyboard and mouse arrive as window
/// events, so a headless host is a process no key can reach. Pressing
/// Play in the editor and then a key did nothing at all (#710).
///
/// So the editor captures from its window and posts snapshots here, and
/// `RemoteInputBackend` turns them back into the same
/// `Box<dyn InputBackend>` a shipped game reads. Project code is
/// identical either way; only who fills the buffer differs.
///
/// # Why this lives in the facade
///
/// Same reason as [`PhysicsRemotePlugin`]: `kooch_remote` knows about
/// entities and deliberately not about input, and `kooch_input` knows
/// about devices and deliberately not about the protocol. They meet
/// here, in the crate that already depends on both.
#[cfg(all(feature = "input", feature = "remote"))]
pub struct InputRemotePlugin;

#[cfg(all(feature = "input", feature = "remote"))]
impl kooch_core::plugin::Plugin for InputRemotePlugin {
    fn build(&self, app: &mut kooch_core::app::App) {
        let backend: Box<dyn kooch_input::InputBackend> =
            Box::new(kooch_input::RemoteInputBackend::new());
        app.insert_resource(backend);
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
                    "input.state",
                    Box::new(|resources, payload| {
                        let snapshot: kooch_input::InputSnapshot =
                            kooch_remote::serde_json::from_value(payload.clone())
                                .map_err(|e| format!("malformed input snapshot: {e}"))?;
                        let backend = resources
                            .get_mut::<Box<dyn kooch_input::InputBackend>>()
                            .ok_or_else(|| "this host has no input backend".to_owned())?;
                        // Through the trait rather than a downcast: a
                        // backend reading real devices takes the default
                        // and ignores it, so this is safe to call on
                        // whichever one the host happens to hold.
                        backend.apply_snapshot(&snapshot);
                        Ok(kooch_remote::serde_json::Value::Null)
                    }),
                );
            },
        );
    }

    fn name(&self) -> &str {
        "InputRemotePlugin"
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

#[cfg(test)]
mod licence_tests;

#[cfg(test)]
mod boot_scene_tests;

#[cfg(test)]
mod engine_assets_tests;
