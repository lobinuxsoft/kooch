//! Wire types for the remote editor protocol.
//!
//! Every payload is plain, serde-serializable data — no engine handles
//! cross the boundary. Components are named by their fully-qualified
//! type path (never [`std::any::TypeId`], which is process-local), and
//! entities by a `(index, generation)` pair, so a client that shares no
//! type table with the server can still address ECS state precisely.
//!
//! The framing is a minimal JSON-RPC: a [`Request`] names a [`Method`]
//! and its parameters; a [`Response`] is either the method's result or a
//! typed [`RemoteError`]. HTTP carries it; see [`crate::server`].

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

/// One component on an entity, named and with its reflected fields.
///
/// Field values reuse [`ReflectValue`], the same type the scene format
/// and the editor's field widgets already serialize.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentSnapshot {
    pub type_name: String,
    pub fields: Vec<(String, ReflectValue)>,
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
    /// The scene this entity was authored in, `None` for one that belongs
    /// to none — an editor helper, or something spawned and not yet saved.
    ///
    /// 🔴 Carried out of band for the same reason `parent` is: membership
    /// lives in `SceneMember`, which is derived on load and never written
    /// to a scene file. It is reflected — a world rebuild has to carry it
    /// — so the host skips it explicitly when listing components, leaving
    /// this the one place it travels. Without this every mirrored entity
    /// arrives belonging to nothing, and since **Open Project always
    /// opens remote**, that is every entity the editor normally shows.
    #[serde(default)]
    pub scene: Option<Guid>,
    pub components: Vec<ComponentSnapshot>,
}

/// One scene the project has open, as the editor needs to list it.
///
/// 🔴 The editor cannot answer this from its own state. Its
/// `SceneManager` seeds an empty scene with a freshly generated `Guid`
/// and no path, while the project holds a different `SceneManager` with
/// the real files under different ids — so the editor was listing a
/// scene that exists nowhere and filing every mirrored entity under
/// "Unsaved", because the scene each one names was not in its list.
///
/// The open set belongs to the project for the same reason the entities
/// do: it is the side that loaded them. Carried per reply rather than
/// behind a method of its own, like [`HostMetrics`] — it is a handful of
/// entries, the editor already pulls a snapshot every frame, and a
/// second round trip is a second thing that can be a frame out of date.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneEntry {
    /// Identity, matching [`EntitySnapshot::scene`] and the scene file's
    /// own `id`.
    pub id: Guid,
    /// Where it was loaded from, or `None` for one never saved.
    ///
    /// A string, not a `PathBuf`: the wire carries no host paths as
    /// types, and the client only ever shows it.
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
    /// The field's doc comment, shown as an Inspector tooltip (#737).
    /// Empty when the field has none.
    ///
    /// 🔴 This is the path that matters. Open Project always opens
    /// remote, so the editor inspects the world over the wire — a
    /// tooltip that only travels the in-process path is one the user
    /// never sees.
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

/// A remote method and its parameters.
///
/// Serialized with an internal `method` tag plus flattened params, so
/// the JSON reads as `{"method": "set_field", "entity": ..., ...}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Method {
    /// Liveness check. Returns [`ResponseData::Pong`].
    Ping,
    /// A method this crate does not know about, served by whichever
    /// subsystem registered it — see [`crate::extensions`].
    ///
    /// The payload is opaque here on purpose: `kooch_remote` depends on
    /// `kooch_core` and `kooch_ecs` and should keep doing so, rather than
    /// growing a dependency on every subsystem that wants to be asked
    /// something.
    Extension {
        /// `subsystem.method`, e.g. `physics.debug_lines`.
        name: String,
        #[serde(default)]
        payload: serde_json::Value,
    },
    /// Every non-ephemeral entity with its components and fields.
    ///
    /// `since` is the revision the caller already holds. When it matches
    /// the one the server last handed out, the reply carries only what
    /// changed; anything else — a fresh client, a missed frame, a
    /// restarted project — gets everything, with `full` set.
    ///
    /// Asking for a diff is not a promise of receiving one. The server
    /// decides, and says which it sent, because a client that assumed
    /// wrong would silently keep entities the project had deleted.
    ListEntities {
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
    /// Spawn a new entity, optionally named. Returns its [`EntityId`].
    ///
    /// Where it lands is asked for, not inferred. Every spawn used to
    /// arrive in the active scene at the root, which is right for a
    /// toolbar button and wrong for a menu opened on a scene, or on an
    /// entity, that is not the active one — the entity appears somewhere
    /// other than where it was asked for, and the only sign is a row in
    /// the wrong group.
    Spawn {
        name: Option<String>,
        /// Which scene to author it into. `None` means the active one.
        ///
        /// Ignored when `parent` is set: an entity's scene is its
        /// parent's, so a parent already answers this, and honouring both
        /// would let a caller ask for a child of an entity in one scene
        /// and a member of another.
        #[serde(default)]
        scene: Option<Guid>,
        /// What to hang it off, or `None` for a root of its scene.
        #[serde(default)]
        parent: Option<EntityId>,
    },
    /// Despawn an entity.
    Despawn { entity: EntityId },
    /// Reparent an entity, or unparent it when `parent` is `None`.
    ///
    /// A method of its own rather than a `SetField` on `Parent`, because
    /// `Parent::reflect_set` is deliberately read-only: an entity handle is
    /// not a reflectable value and `ReflectValue` has no variant for one.
    /// `Parent.entity` reflects *out* as an `"index:generation"` string for
    /// display and cannot be written back.
    SetParent {
        entity: EntityId,
        /// `None` unparents to the scene root — the same operation, so it
        /// does not get a second method.
        parent: Option<EntityId>,
    },
    /// Persist one open scene to a file on the server's disk.
    ///
    /// 🔴 One scene, not the world. This used to write
    /// `SceneDocument::from_ecs` — every entity alive, under a freshly
    /// generated document id. With two scenes open that put both scenes'
    /// entities in one file, so the next load spawned everything twice,
    /// and the id changed on every save, breaking whatever named the
    /// scene. The engine has always had `from_ecs_scene`; the local
    /// editor path used it and this one did not, and **Open Project
    /// always opens remote**.
    SaveScene {
        path: String,
        /// Which scene to write. `None` means the active one — what a
        /// client that knows of only one scene sends, and what a host
        /// older than this field is asked for anyway.
        #[serde(default)]
        scene: Option<Guid>,
    },
    /// Write one entity and its descendants to a scene file — a prefab.
    ///
    /// Server-side because the world it captures lives here; the editor's
    /// mirror is a projection and is not what should be written to disk.
    SavePrefab { entity: EntityId, path: String },
    /// Tell the project an asset file on disk was written.
    ///
    /// The project caches what it loads and the editor writes those files,
    /// so without this the project keeps using the version it read first —
    /// a value held in two places, the exact thing the reference model
    /// exists to avoid.
    ///
    /// Any asset, not only a prefab: a material, an input action and a
    /// mesh all go stale the same way, and the editor cannot know which
    /// types the project happens to have loaded a given path under. It
    /// also covers a file that is new, which the project has no identity
    /// for until it is told.
    ReloadAsset { path: String },
    /// Stamp a prefab file into the live ECS, returning its root.
    ///
    /// Distinct from [`Self::LoadScene`], which *replaces* the world. This
    /// adds to it, with identity remapped so the same file can be
    /// instanced more than once.
    ///
    /// No position parameter: the root comes back, so placing it is a
    /// `SetField` on its `Transform`. The wire format carries no spatial
    /// types of its own — everything spatial travels as a `ReflectValue`,
    /// and a second way to move an entity is a second thing to keep in step
    /// with the first.
    InstantiatePrefab { path: String },
    /// Move an entity among its siblings: under `parent`, before
    /// `before`.
    ///
    /// One method rather than a reparent plus a field write, because the
    /// numbering policy lives in the engine (`kooch_ecs::order::place`)
    /// and a client computing it would have to renumber a sibling group
    /// over the wire, one round trip per entity.
    MoveEntity {
        entity: EntityId,
        /// `None` makes it a root of its scene.
        #[serde(default)]
        parent: Option<EntityId>,
        /// The sibling it goes in front of; `None` puts it last.
        #[serde(default)]
        before: Option<EntityId>,
    },
    /// Throw away one open scene's edits and read it back from its file.
    ///
    /// Only that scene: the others keep their edits. `None` reverts the
    /// active one.
    RevertScene {
        #[serde(default)]
        scene: Option<Guid>,
    },
    /// Open an empty unsaved scene beside the ones already loaded, and
    /// make it active. Returns its identity as [`ResponseData::SceneOpened`].
    ///
    /// "Start something new" while a world is already open. An entity has
    /// to belong to a scene, so creating one is what makes "put this
    /// somewhere of its own" answerable — which is what right-clicking
    /// the World panel's empty space means.
    NewScene,
    /// Replace the live ECS with a scene file from the server's disk.
    LoadScene { path: String },
    /// Start or stop the project's gameplay systems in place.
    ///
    /// Starting snapshots the world first and stopping restores that
    /// snapshot, so a play session leaves the authored scene untouched.
    SetPlaying { playing: bool },
}

/// A request: a method invocation carrying a client-chosen id, echoed in
/// the response so a client can match replies to calls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Correlation id, echoed back verbatim.
    #[serde(default)]
    pub id: u64,
    #[serde(flatten)]
    pub method: Method,
}

/// The successful result of a method, tagged by result kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResponseData {
    /// Reply to [`Method::Ping`].
    Pong,
    /// Reply to [`Method::ListEntities`].
    ///
    /// `entities` is the whole world when `full`, and only what changed
    /// otherwise. `removed` is always the entities that went away since
    /// the caller's revision — empty in a full reply, since absence
    /// already says it.
    Entities {
        entities: Vec<EntitySnapshot>,
        /// Entities that no longer exist. Only meaningful in a diff.
        #[serde(default)]
        removed: Vec<EntityId>,
        /// The revision this reply brings the caller to. Pass it back as
        /// `since` on the next call.
        #[serde(default)]
        revision: u64,
        /// Whether `entities` is the entire world. A client must replace
        /// its mirror wholesale when this is set, rather than merging —
        /// merging a full reply into a stale mirror keeps whatever the
        /// full reply omitted.
        #[serde(default)]
        full: bool,
        /// What the host's own frame cost, when it is measuring one.
        ///
        /// `None` from a host that predates this field, which is why it
        /// is an `Option` and not a zeroed struct: a zero would render as
        /// "the project runs infinitely fast" rather than as "nobody
        /// said".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host: Option<HostMetrics>,
        /// Which scenes the project has open.
        ///
        /// `None` means nobody said — an older host, or one with no
        /// `SceneManager` — and the client should keep whatever it was
        /// showing. `Some` is the whole open set, replacing it.
        ///
        /// The distinction is the point: an empty `Vec` would be
        /// indistinguishable from a host that never sent the field, and
        /// the editor would blank a list it had no news about.
        ///
        /// Sent whole every reply rather than diffed like `entities`.
        /// There are as many of these as a person has scenes open, and
        /// a diff of three entries costs more to be right about than to
        /// resend.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scenes: Option<Vec<SceneEntry>>,
    },
    /// Reply to [`Method::GetSchema`].
    Schema { components: Vec<ComponentSchema> },
    /// Reply to [`Method::Spawn`] — the new entity's handle.
    Spawned { entity: EntityId },
    /// Reply to [`Method::NewScene`] — the new scene's identity.
    SceneOpened { scene: Guid },
    /// Reply to any method that mutates but returns nothing.
    Ok,
    /// Reply to [`Method::Extension`] — whatever the handler returned,
    /// uninterpreted.
    Extension {
        name: String,
        result: serde_json::Value,
    },
}

/// What the project's process costs per frame.
///
/// Rides along with the world snapshot the editor already pulls every
/// frame, so it needs no request of its own and no second round trip.
///
/// # These are not frames per second in the rendering sense
///
/// A remote host has no window and no renderer — `RemoteHostPlugins`
/// draws nothing. What it has is a simulation tick: ECS, physics,
/// gravity, camera rigs. That is what these describe, and calling it FPS
/// would be a lie drawn in a nice font.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HostMetrics {
    /// Wall-clock milliseconds between tick starts, waiting included.
    pub frame_ms: f32,
    /// Milliseconds of work in the tick, waiting excluded.
    pub cpu_frame_ms: f32,
    /// Ticks per second from the last tick alone.
    ///
    /// Sent beside the average rather than left to be derived from
    /// `frame_ms`: the two are read side by side, and a rate computed
    /// from one field next to an average computed from sixty is how
    /// `23 /s` ends up printed next to `513.99 ms`.
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
    /// No subsystem on this host registered that extension.
    ///
    /// Usually a feature that is off rather than a mistake: a host built
    /// without physics serves no `physics.*`, and a client should be able
    /// to tell that from a handler that ran and failed.
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
