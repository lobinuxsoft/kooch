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

use serde::{Deserialize, Serialize};

use ome_ecs::entity::Entity;
use ome_ecs::reflect::ReflectValue;

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
    pub components: Vec<ComponentSnapshot>,
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
    /// Every non-ephemeral entity with its components and fields.
    ListEntities,
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
    Spawn { name: Option<String> },
    /// Despawn an entity.
    Despawn { entity: EntityId },
    /// Persist the live ECS to a scene file on the server's disk.
    SaveScene { path: String },
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
    Entities { entities: Vec<EntitySnapshot> },
    /// Reply to [`Method::GetSchema`].
    Schema { components: Vec<ComponentSchema> },
    /// Reply to [`Method::Spawn`] — the new entity's handle.
    Spawned { entity: EntityId },
    /// Reply to any method that mutates but returns nothing.
    Ok,
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
mod tests {
    use super::*;

    #[test]
    fn entity_id_round_trips_through_entity() {
        let e = Entity::new(7, 3);
        let id: EntityId = e.into();
        assert_eq!(id.index, 7);
        assert_eq!(id.generation, 3);
        assert_eq!(Entity::from(id), e);
    }

    #[test]
    fn request_deserializes_from_flat_json() {
        let json = r#"{"id":5,"method":"set_field","entity":{"index":1,"generation":0},"component":"game::Health","field":"hp","value":{"U32":42}}"#;
        let req: Request = serde_json::from_str(json).unwrap();
        assert_eq!(req.id, 5);
        match req.method {
            Method::SetField {
                entity,
                component,
                field,
                value,
            } => {
                assert_eq!(entity.index, 1);
                assert_eq!(component, "game::Health");
                assert_eq!(field, "hp");
                assert_eq!(value, ReflectValue::U32(42));
            }
            other => panic!("wrong method: {other:?}"),
        }
    }

    #[test]
    fn ping_request_needs_no_params() {
        let req: Request = serde_json::from_str(r#"{"id":1,"method":"ping"}"#).unwrap();
        assert_eq!(req.method, Method::Ping);
    }

    #[test]
    fn response_round_trips() {
        let resp = Response::ok(
            9,
            ResponseData::Spawned {
                entity: EntityId {
                    index: 2,
                    generation: 1,
                },
            },
        );
        let json = serde_json::to_string(&resp).unwrap();
        let back: Response = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn error_response_round_trips() {
        let resp = Response::err(
            3,
            RemoteError::UnknownComponent {
                type_name: "game::Missing".into(),
            },
        );
        let json = serde_json::to_string(&resp).unwrap();
        let back: Response = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, back);
    }
}
