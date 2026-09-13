//! Wire types for the remote editor protocol: plain serde data, components named by type path and
//! entities by `(index, generation)`, never process-local handles.
//! A minimal JSON-RPC over a local socket, one object per line; see [`crate::server`].

use kooch_core::Guid;
use serde::{Deserialize, Serialize};

use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::ReflectValue;

/// A stable, serializable entity handle: the live entity's index and
/// generation. Survives the wire without exposing the engine's
/// [`Entity`] type to the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntityId {
    pub index: u32,
    pub generation: u32,
}

impl From<Entity> for EntityId {
    fn from(e: Entity) -> Self {
        Self {
            index: e.index(),
            generation: e.generation(),
        }
    }
}

impl From<EntityId> for Entity {
    fn from(id: EntityId) -> Self {
        Entity::new(id.index, id.generation)
    }
}

/// One component on an entity with its reflected fields, as [`ReflectValue`]s — the scene format's
/// and the editor widgets' type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentSnapshot {
    pub type_name: String,
    pub fields: Vec<(String, ReflectValue)>,
}

/// One entity's local transform as matrix columns — the mirror writes `Transform`, so no rebuild
/// from three fields.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MovedTransform {
    pub id: EntityId,
    pub matrix: [f32; 16],
}

/// One entity with its name and components, as seen over the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntitySnapshot {
    pub id: EntityId,
    /// The `Name` component's value, if any — a display convenience so
    /// the client need not dig through `components` for the label.
    pub name: Option<String>,
    /// Parent entity, for hierarchy reconstruction on the client.
    pub parent: Option<EntityId>,
    /// The entity's scene, `None` for none. 🔴 Out of band like `parent`: membership is derived on
    /// load and skipped when listing components, and Open Project always opens remote.
    #[serde(default)]
    pub scene: Option<Guid>,
    pub components: Vec<ComponentSnapshot>,
}

/// One scene the project has open, as the editor lists it. 🔴 Only the project can say: the editor's
/// own manager holds an unsaved scene with a random id. Sent with each reply, like [`HostMetrics`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneEntry {
    /// Identity, matching [`EntitySnapshot::scene`] and the scene file's
    /// own `id`.
    pub id: Guid,
    /// Where it was loaded from, or `None` if never saved — a string, since the wire carries no
    /// host paths as types.
    pub path: Option<String>,
    /// Whether new entities are authored into it.
    pub active: bool,
    /// Whether it has edits not on disk.
    pub dirty: bool,
}

/// Static metadata for one field of a registered component type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldSchema {
    pub name: String,
    pub type_name: String,
    /// Allowed values for enum-like fields, empty otherwise. Lets the
    /// client render a dropdown without knowing the Rust type.
    pub choices: Vec<String>,
    /// Canonical asset type this field references, empty if it is not an
    /// asset reference.
    pub asset_type: String,
    /// The field's doc comment, an Inspector tooltip (#737), empty when none. 🔴 Open Project always
    /// opens remote, so this is the path users see.
    #[serde(default)]
    pub doc: String,
}

/// Schema for one registered component type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentSchema {
    pub type_name: String,
    /// `None` when the type has no reflection (cannot be inspected or
    /// edited field-by-field), `Some` with its field layout otherwise.
    pub fields: Option<Vec<FieldSchema>>,
    /// Category tag from `#[reflect(category = "...")]`, for grouping in
    /// the Add Component menu.
    pub category: Option<String>,
}

/// A remote method and its parameters, serialised as an internal `method` tag with flattened
/// params.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Method {
    /// Liveness check. Returns [`ResponseData::Pong`].
    Ping,
    /// A method this crate does not know, served by whichever subsystem registered it (see
    /// [`crate::extensions`]), opaque so `kooch_remote` depends on no subsystem.
    Extension {
        /// `subsystem.method`, e.g. `physics.debug_lines`.
        name: String,
        #[serde(default)]
        payload: serde_json::Value,
    },
    /// Every non-ephemeral entity with its components. With a matching `since` the reply holds only
    /// changes; otherwise everything, with `full` — the server decides and says which.
    ListEntities {
        #[serde(default)]
        since: Option<u64>,
    },
    /// The transforms that moved since `since` (#1012) — a separate method because it reads one
    /// component instead of reflecting the world (38.9 ms on 2159 entities). Used while playing.
    ListMoved {
        #[serde(default)]
        since: Option<u64>,
    },
    /// Every registered component type with its field schema.
    GetSchema,
    /// Overwrite one field of one component on one entity.
    SetField {
        entity: EntityId,
        component: String,
        field: String,
        value: ReflectValue,
    },
    /// Add a default-constructed component to an entity.
    AddComponent { entity: EntityId, component: String },
    /// Remove a component from an entity.
    RemoveComponent { entity: EntityId, component: String },
    /// Spawn a new entity, optionally named, returning its [`EntityId`] — placed where asked, not
    /// always at the active scene's root.
    Spawn {
        name: Option<String>,
        /// Which scene to author it into, `None` for the active one; ignored when `parent` is set,
        /// since a parent decides the scene.
        #[serde(default)]
        scene: Option<Guid>,
        /// What to hang it off, or `None` for a root of its scene.
        #[serde(default)]
        parent: Option<EntityId>,
    },
    /// Despawn an entity.
    Despawn { entity: EntityId },
    /// Reparent an entity, or unparent with `None` — its own method because `Parent` is read-only
    /// through reflection.
    SetParent {
        entity: EntityId,
        /// `None` unparents to the scene root — the same operation, so it
        /// does not get a second method.
        parent: Option<EntityId>,
    },
    /// Persist one open scene to the server's disk. 🔴 One scene, not the world, under its own id —
    /// writing the world duplicated entities on the next load.
    SaveScene {
        path: String,
        /// Which scene to write. `None` means the active one — what a
        /// client that knows of only one scene sends, and what a host
        /// older than this field is asked for anyway.
        #[serde(default)]
        scene: Option<Guid>,
    },
    /// Write one entity and its descendants to a scene file — a prefab — server-side, where the
    /// real world lives.
    SavePrefab { entity: EntityId, path: String },
    /// Tell the project an asset file was written — any type, new or not — so it stops using the
    /// version it read first.
    ReloadAsset { path: String },
    /// Stamp a prefab into the live ECS with remapped identity, returning its root; unlike
    /// [`Self::LoadScene`] it adds. Position it with a `SetField`.
    InstantiatePrefab { path: String },
    /// Move an entity among its siblings, under `parent` before `before` — one call, since the
    /// numbering policy lives in the engine.
    MoveEntity {
        entity: EntityId,
        /// `None` makes it a root of its scene.
        #[serde(default)]
        parent: Option<EntityId>,
        /// The sibling it goes in front of; `None` puts it last.
        #[serde(default)]
        before: Option<EntityId>,
    },
    /// Throw away one open scene's edits and reread it; the others keep theirs. `None` reverts the
    /// active one.
    RevertScene {
        #[serde(default)]
        scene: Option<Guid>,
    },
    /// Open an empty unsaved scene beside the loaded ones and make it active, returning
    /// [`ResponseData::SceneOpened`] — the World panel's empty-space gesture.
    NewScene,
    /// Replace the live ECS with a scene file from the server's disk.
    LoadScene { path: String },
    /// Close one open scene, despawning only its entities. 🔴 The open set is the project's; closing
    /// it in the editor's view closed nothing.
    CloseScene { scene: Guid },
    /// Make an already-open scene the one new entities are authored into.
    SetActiveScene { scene: Guid },
    /// Open a scene file beside what is loaded and make it active, returning
    /// [`ResponseData::SceneOpened`]. 🔴 Unlike [`Self::LoadScene`], it adds, where the world lives.
    LoadSceneAdditive { path: String },
    /// Start or stop gameplay in place; stopping restores the snapshot taken on start.
    SetPlaying { playing: bool },

    /// Every system the project schedules, in frame order — the editor cannot read another
    /// process's schedule.
    ListSystems,

    /// Stop or restart one system from the next frame, addressed by name and occurrence, since
    /// indices shift and closures share names.
    SetSystemEnabled {
        name: String,
        nth: u32,
        enabled: bool,
    },
}

/// One scheduled system, as the editor's panel shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemEntry {
    /// The stage's name, not its discriminant: the wire outlives any one
    /// build's enum, and a number would silently mean something else the
    /// day a stage is inserted.
    pub stage: String,
    /// The full path, which is what a log or a profile says.
    pub name: String,
    /// What to put in the list, with the wrapper and the modules off.
    pub short: String,
    /// Which occurrence of `name` this is; 0 unless the name repeats.
    pub nth: u32,
    /// `true` when the project scheduled it, `false` for the engine.
    pub project: bool,
    pub gpu: bool,
    pub enabled: bool,
}

/// A request: a method invocation carrying a client-chosen id, echoed in
/// the response so a client can match replies to calls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Correlation id, echoed back verbatim.
    #[serde(default)]
    pub id: u64,
    /// Whether the sender has stopped listening (#1015). 🔴 A reply nobody reads still held the
    /// single listener for a host frame — 41 ms of a 49 ms editor frame while a key was down.
    /// Defaults `false`.
    #[serde(default)]
    pub notify: bool,
    #[serde(flatten)]
    pub method: Method,
}

/// The successful result of a method, tagged by result kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResponseData {
    /// Reply to [`Method::Ping`].
    Pong,
    /// Reply to [`Method::ListMoved`]; `full` means the entity set changed, so the caller pulls a
    /// whole [`Method::ListEntities`] that frame.
    Moved {
        moved: Vec<MovedTransform>,
        #[serde(default)]
        removed: Vec<EntityId>,
        revision: u64,
        full: bool,
        #[serde(default)]
        host: Option<HostMetrics>,
    },
    /// Reply to [`Method::ListEntities`]: the whole world when `full`, otherwise only changes;
    /// `removed` lists what went away since the caller's revision.
    Entities {
        entities: Vec<EntitySnapshot>,
        /// Entities that no longer exist. Only meaningful in a diff.
        #[serde(default)]
        removed: Vec<EntityId>,
        /// The revision this reply brings the caller to. Pass it back as
        /// `since` on the next call.
        #[serde(default)]
        revision: u64,
        /// Whether `entities` is the whole world: the client must replace its mirror, not merge
        /// into it.
        #[serde(default)]
        full: bool,
        /// The host's frame cost when it measures one — `None` from older hosts, not a zero that
        /// reads as infinitely fast.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host: Option<HostMetrics>,
        /// Which scenes the project has open: `None` means nobody said, keep what you show; `Some`
        /// replaces the set. Sent whole — a few entries.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scenes: Option<Vec<SceneEntry>>,
    },
    /// Reply to [`Method::GetSchema`].
    Schema { components: Vec<ComponentSchema> },
    /// Reply to [`Method::Spawn`] — the new entity's handle.
    Spawned { entity: EntityId },
    /// Reply to [`Method::NewScene`] — the new scene's identity.
    SceneOpened { scene: Guid },
    /// Reply to [`Method::ListSystems`], in the order a frame runs them.
    Systems { systems: Vec<SystemEntry> },
    /// Reply to any method that mutates but returns nothing.
    Ok,
    /// Reply to [`Method::Extension`] — whatever the handler returned,
    /// uninterpreted.
    Extension {
        name: String,
        result: serde_json::Value,
    },
}

/// What the project's process costs per frame, riding the snapshot already pulled. Simulation
/// ticks, not FPS — a host renders nothing.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HostMetrics {
    /// Wall-clock milliseconds between tick starts, waiting included.
    pub frame_ms: f32,
    /// Milliseconds of work in the tick, waiting excluded.
    pub cpu_frame_ms: f32,
    /// Ticks per second from the last tick alone, sent beside the average so the two read
    /// consistently.
    pub ticks_instant: f32,
    /// Ticks per second, averaged over the host's own window.
    pub ticks_per_second: f32,
}

/// A typed failure. Serialized as `{"error": {...}}` in [`Response`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum RemoteError {
    /// The request body was not valid protocol JSON.
    BadRequest { detail: String },
    /// The named entity is not alive.
    NoSuchEntity { entity: EntityId },
    /// The server binary has no Rust type for this component name.
    UnknownComponent { type_name: String },
    /// A field write failed (no such field, or a type mismatch).
    FieldError { detail: String },
    /// A scene save/load failed.
    SceneError { detail: String },
    /// The method ran but the ECS was not available (e.g. no registry).
    Unavailable { detail: String },
    /// No subsystem on this host registered that extension — usually a feature that is off,
    /// distinguishable from a handler that failed.
    UnknownExtension { name: String },
    /// The extension ran and reported its own failure.
    ExtensionFailed { name: String, detail: String },
}

/// A response: the echoed request id plus either a result or an error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(flatten)]
    pub payload: ResponsePayload,
}

/// Either a method result or a typed error — flattened into [`Response`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponsePayload {
    Result(ResponseData),
    Error(RemoteError),
}

impl Response {
    /// Builds a success response for `id`.
    pub fn ok(id: u64, data: ResponseData) -> Self {
        Self {
            id,
            payload: ResponsePayload::Result(data),
        }
    }

    /// Builds an error response for `id`.
    pub fn err(id: u64, error: RemoteError) -> Self {
        Self {
            id,
            payload: ResponsePayload::Error(error),
        }
    }
}

#[cfg(test)]
mod tests;
