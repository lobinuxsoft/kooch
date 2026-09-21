//! Kóoch, the GPU-driven game engine: this facade re-exports the engine crates behind Cargo
//! features (see `[features]` in `Cargo.toml`). Default: `window`, `render`, `gizmos`, `world`,
//! `input`.

/// The engine's licence, verbatim. 🔴 Compiled into every binary that links the engine as an `rlib`,
/// so shipping without it means not using the engine; the source is protected by licence, not
/// hiding (#754).
pub const LICENSE: &str = include_str!("../LICENSE.md");

// Named `profiler` and not `profiling` on purpose: a module of that name
// in the crate root shadows the `profiling` facade crate for every path
// written in this file.
#[cfg(all(feature = "physics", feature = "render"))]
pub mod collider_meshes;
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

#[cfg(feature = "blockmesh")]
pub use kooch_blockmesh::{Block, BlockMesh, BlockPlugin, BuiltBlocks};
#[cfg(feature = "character")]
pub use kooch_character;
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

    // The maths types of every component, re-exported: the lock holds glam 0.29–0.33, and a project
    // adding its own can pick a different one (#657).
    pub use glam;
    pub use glam::{Mat3, Mat4, Quat, Vec2, Vec3, Vec4};

    // Logging, for the same reason: a game writing to the editor's Console should not match the
    // engine's `tracing` version itself.
    pub use tracing;
    pub use tracing::{debug, error, info, warn};

    // The type of every asset reference, named the first time a component points at a mesh or
    // prefab.
    pub use kooch_core::Guid;

    // What a game touches on day one. The prelude is the discovery surface
    // (`docs/CAPABILITIES.md`): missing from it reads like never built.
    pub use kooch_ecs::{
        Children, Commands, Component, ComponentId, ComponentRegistry, ComponentStorage, EcsPlugin,
        Entity, EntityAllocator, GlobalTransform, MeshRenderer, Name, OrthographicCamera, Parent,
        PerspectiveCamera, Reflect, SceneManager, Transform,
    };
    // Where a system binds into the frame, said at the system. Inert:
    // the editor's codegen reads it, the compiler passes the function
    // through untouched.
    pub use kooch_ecs::system;
    // The rest of what a scene is made of: what lights it, what the sky
    // is, and the override that pins an entity's level of detail.
    pub use kooch_ecs::{DirectionalLight, LodForceLevel, PointLight, SkyRenderer, SpotLight};
    // Iterating entities by the components they carry, instead of asking
    // the registry for one storage at a time and joining by hand — which
    // is what three engine crates still do, 37 times over.
    pub use kooch_ecs::{Query, With, Without};

    #[cfg(feature = "input")]
    pub use kooch_input::{
        CursorMode, InputBackend, InputPlugin, KeyCode, MouseButton,
        backend::{GamepadAxis, GamepadButton, GamepadId},
        ids::MouseAxis,
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
        Collider, Joint, PhysicsBody, PhysicsPlugin, PhysicsWorld, PointHit, QueryFilter, RayHit,
        ShapeAt, ShapeHit, SolverBody,
    };
    // Editor-built level pieces, nameable so a game can read them; `BuiltBlocks` says a shape
    // changed.
    #[cfg(feature = "blockmesh")]
    pub use kooch_blockmesh::{Block, BlockMesh, BlockPlugin, BuiltBlocks};

    // 🔴 Goals, checkpoints and death planes are sensors, and the event type could not be named
    // here. Read with `Events<CollisionStarted>`; ⚠️ one frame late by design, since `Events` is
    // double-buffered.
    #[cfg(feature = "physics")]
    pub use kooch_physics::plugin::{CollisionStarted, CollisionStopped, ContactForce, JointBroke};

    // `gravity_at` answers "which way is down here", and is the only
    // honest way to ask it: a controller that works it out differently
    // from the solver ends up disagreeing about where the floor is.
    #[cfg(feature = "gravity")]
    pub use kooch_gravity::{
        AreaGravity, BoxGravity, GlobalGravity, GravityPlugin, GravityPriority, PlaneGravity,
        PointGravity, gravity_at, gravity_dominant, gravity_up,
    };

    // `Grounded` because jumping, animation and footsteps all read it; `Facing` because gameplay
    // steers and the controller cannot know where.
    #[cfg(feature = "character")]
    pub use kooch_character::{
        CharacterController, CharacterPlugin, Facing, Grounded, Sprint, Touching, Walk, WallJump,
        WallRun, WallSlide,
    };
    // Not `Jump`: a project that already has one of its own — and
    // `roll-a-ball` does, for a ball that is not a character — would
    // find the two names colliding on the same `use`.
    #[cfg(feature = "character")]
    pub use kooch_character::jump::Jump as CharacterJump;

    // The mode constants come along: without them `VirtualCamera` cannot
    // be configured from code at all, and `UP_GRAVITY` is what makes a
    // camera work while orbiting a planet.
    #[cfg(feature = "camera")]
    pub use kooch_camera::{
        CameraBrain, CameraPlugin, CameraTarget, FOLLOW_GLUED, FOLLOW_NONE, FOLLOW_SIMPLE,
        FOLLOW_THIRD_PERSON, LOOK_AT_MIMIC, LOOK_AT_NONE, LOOK_AT_SIMPLE, UP_GRAVITY, UP_TARGET,
        UP_WORLD, VirtualCamera,
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

    // 🔴 A shipped game's assets are in a pack, so `<exe>/assets` is right though no such directory
    // exists — `.exists()` would fall back to the cwd, the user's home.
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

    // No loader list: each asset type registers itself with `kooch_core::register_asset!`, so the
    // editor keeps no drifting second copy.
    let mut plugin = kooch_render::plugin::AssetPlugin::new().with_root(primary);
    if let Some((pack, key)) = shipped {
        tracing::info!(target: "kooch::shipped", path = %pack.display(), "reading assets from the shipped pack");
        // 🔴 Mounted over the game folder: the pack holds `assets/` and `scenes/`, since a plain-RON
        // scene beside an encrypted pack would publish the design.
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

/// Plugins for a **remote authoring host** (`cargo run -- --remote`): [`DefaultPlugins`] minus
/// window and renderer, answering the editor over a local socket. A headless asset plugin resolves
/// prefab guids; eager import stays off.
pub struct RemoteHostPlugins;

impl kooch_core::plugin::PluginGroup for RemoteHostPlugins {
    fn build(self) -> kooch_core::plugin::PluginGroupBuilder {
        let builder = kooch_core::plugin::PluginGroupBuilder::new()
            .add(kooch_core::plugin::CorePlugin)
            .add(kooch_ecs::EcsPlugin);

        // 🔴 Gated like `DefaultPlugins`: `AssetPlugin` lives in `kooch_render`, and a headless host
        // is exactly what drops `render` (#686).
        #[cfg(feature = "render")]
        let builder = builder.add(default_asset_plugin().headless());

        // The host is what actually simulates when the editor presses
        // Play, so it needs physics even though it draws nothing.
        #[cfg(feature = "physics")]
        let builder = builder.add(kooch_physics::PhysicsPlugin::new());

        // Mesh-derived colliders. Needs both halves, which is why it is
        // added from the facade rather than from either crate.
        #[cfg(all(feature = "physics", feature = "render"))]
        let builder = builder.add(crate::collider_meshes::ColliderMeshPlugin);

        // Editor-authored blocks (#946), added here: the editor spawns them over the wire, and a
        // forgotten line reads only "add_component failed".
        #[cfg(feature = "blockmesh")]
        let builder = builder.add(kooch_blockmesh::BlockPlugin);

        // Gravity that points somewhere other than down. Inert until a
        // scene holds a source, so adding it changes nothing on its own.
        #[cfg(all(feature = "physics", feature = "gravity"))]
        let builder = builder.add(kooch_gravity::GravityPlugin);

        // After gravity, always: the spring cancels *this* step's pull,
        // and cancelling last step's is a character that sinks whenever
        // the field changes under it.
        #[cfg(feature = "character")]
        let builder = builder.add(kooch_character::CharacterPlugin);

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

        // First, so the socket listens while loaders do the slowest work, and nobody edits a game
        // to profile it. Absent without the feature.
        #[cfg(feature = "profiling")]
        let builder = builder.add(crate::profiler::ProfilingPlugin::default());

        #[cfg(all(feature = "physics", feature = "gravity"))]
        let builder = builder.add(kooch_gravity::GravityPlugin);

        // After gravity, always: the spring cancels *this* step's pull,
        // and cancelling last step's is a character that sinks whenever
        // the field changes under it.
        #[cfg(feature = "character")]
        let builder = builder.add(kooch_character::CharacterPlugin);

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

        // Mesh-derived colliders. Needs both halves, which is why it is
        // added from the facade rather than from either crate.
        #[cfg(all(feature = "physics", feature = "render"))]
        let builder = builder.add(crate::collider_meshes::ColliderMeshPlugin);

        // Editor-authored blocks (#946), added here: the editor spawns them over the wire, and a
        // forgotten line reads only "add_component failed".
        #[cfg(feature = "blockmesh")]
        let builder = builder.add(kooch_blockmesh::BlockPlugin);

        // Gravity that points somewhere other than down. Inert until a
        // scene holds a source, so adding it changes nothing on its own.
        #[cfg(all(feature = "physics", feature = "gravity"))]
        let builder = builder.add(kooch_gravity::GravityPlugin);

        // After gravity, always: the spring cancels *this* step's pull,
        // and cancelling last step's is a character that sinks whenever
        // the field changes under it.
        #[cfg(feature = "character")]
        let builder = builder.add(kooch_character::CharacterPlugin);

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

/// Serves `physics.debug_lines` so the editor can ask the host for the solver's own state (#634).
/// Here because `kooch_remote` knows no physics and `kooch_physics` no wires; the extension
/// registry makes it a plugin.
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

/// Receives the input the editor captured: a windowless host gets no key events, so Play plus a key
/// did nothing (#710). `RemoteInputBackend` feeds the same `InputBackend` a game reads; here for
/// the same reason as [`PhysicsRemotePlugin`].
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
                        // Through the trait, not a downcast: a real-device backend ignores
                        // snapshots by default.
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

/// Reads the five switches; a missing one is off, so clients name only what they want and newer
/// editors degrade gracefully.
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

#[cfg(all(test, feature = "physics"))]
mod goal_tests;
