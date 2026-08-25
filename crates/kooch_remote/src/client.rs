//! Blocking client for the remote editor protocol.
//!
//! The editor calls this to drive a project's running ECS. It is the
//! mirror of [`server`](crate::server) and speaks the same
//! [`protocol`](crate::protocol) types, so a request built here
//! deserializes there and back with no schema drift.
//!
//! The transport is a **local socket** — a Unix domain socket or a
//! Windows named pipe — carrying one JSON object per line. It is not a
//! TCP port, which is the point: a port is reachable by anything on the
//! machine, including a web page, and this protocol can write files
//! (#647). A local socket is not addressable from a browser at all.
//!
//! One connection per request, as before. That is more than a local
//! socket needs and is worth revisiting, but changing the transport and
//! the connection lifetime at once would make a regression impossible to
//! attribute.

use std::io::{BufRead, BufReader, Write};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use interprocess::local_socket::traits::Stream as _;
use interprocess::local_socket::{GenericNamespaced, Stream, ToNsName};

use kooch_ecs::reflect::ReflectValue;

use crate::protocol::{
    ComponentSchema, EntityId, EntitySnapshot, Method, RemoteError, Request, Response,
    ResponseData, ResponsePayload,
};

/// A failure talking to the remote server.
#[derive(Debug)]
pub enum ClientError {
    /// The socket could not be reached or the exchange was cut short.
    Io(std::io::Error),
    /// The response body was not valid protocol JSON.
    Decode(String),
    /// The server ran the method and returned a typed failure.
    Remote(RemoteError),
    /// The server answered, but with a result kind this call did not
    /// expect (a protocol mismatch, not a user error).
    Unexpected(ResponseData),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "remote transport error: {e}"),
            Self::Decode(e) => write!(f, "remote decode error: {e}"),
            Self::Remote(e) => write!(f, "remote returned an error: {e:?}"),
            Self::Unexpected(d) => write!(f, "remote returned an unexpected result: {d:?}"),
        }
    }
}

impl std::error::Error for ClientError {}

impl From<std::io::Error> for ClientError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// What the most recent [`RemoteClient::call`] spent, split by where.
///
/// The split is the whole point. A call's total says the editor stalled;
/// it does not say why, and the two causes have unrelated fixes:
///
/// - **Transport dominating** — the wait is not the loopback socket, it
///   is the server's main thread. [`crate::plugin`] answers queued
///   requests from a `Stage::First` system, so a caller blocks until the
///   project reaches its next frame boundary.
/// - **Decode dominating** — the payload itself is too big, and the
///   answer is to send less of it rather than to send it elsewhere.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct CallStats {
    /// Microseconds from opening the socket to holding the reply text.
    pub transport_us: u32,
    /// Microseconds spent parsing that text into a [`Response`].
    pub decode_us: u32,
    /// Bytes of response body, headers excluded.
    pub response_bytes: u32,
}

/// [`CallStats`] as atomics, so recording one keeps [`RemoteClient`]
/// `Sync` without a lock — callers hold it behind `&self`.
///
/// `Relaxed` throughout: the three fields are a diagnostic read once a
/// frame by a HUD, not a synchronisation edge. A torn read across a
/// call boundary would mix two adjacent samples of the same metric,
/// which is invisible at HUD resolution and not worth an `Acquire`.
#[derive(Default)]
struct CallStatsCell {
    transport_us: AtomicU32,
    decode_us: AtomicU32,
    response_bytes: AtomicU32,
}

impl CallStatsCell {
    /// Saturating, because a `u32` of microseconds runs out at 71
    /// minutes and a stuck socket should read as "enormous", not wrap
    /// to nearly zero.
    fn store(&self, transport: Duration, decode: Duration, response_bytes: usize) {
        let us = |d: Duration| d.as_micros().min(u32::MAX as u128) as u32;
        self.transport_us.store(us(transport), Ordering::Relaxed);
        self.decode_us.store(us(decode), Ordering::Relaxed);
        self.response_bytes.store(
            response_bytes.min(u32::MAX as usize) as u32,
            Ordering::Relaxed,
        );
    }

    fn load(&self) -> CallStats {
        CallStats {
            transport_us: self.transport_us.load(Ordering::Relaxed),
            decode_us: self.decode_us.load(Ordering::Relaxed),
            response_bytes: self.response_bytes.load(Ordering::Relaxed),
        }
    }
}

/// Connection details for a running project's remote server.
///
/// Cheap to hold; each call opens its own short-lived connection, so a
/// [`RemoteClient`] is just a socket name plus a request-id counter.
pub struct RemoteClient {
    /// Name of the local socket the project is listening on.
    name: String,
    /// Monotonic request ids, so a reply can be matched to its call.
    next_id: AtomicU64,
    /// Cost of the last completed call, for the editor's perf HUD.
    last_call: CallStatsCell,
}

impl RemoteClient {
    /// A client for the project listening on the local socket `name`.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            next_id: AtomicU64::new(1),
            last_call: CallStatsCell::default(),
        }
    }

    /// The socket name this client talks to.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What the last completed [`Self::call`] cost, split by transport
    /// and decode. All zeroes before the first call completes; a call
    /// that fails leaves the previous sample standing rather than
    /// reporting a zero that reads like "free".
    pub fn last_call_stats(&self) -> CallStats {
        self.last_call.load()
    }

    /// Liveness probe. `Ok(())` means the server answered a ping.
    ///
    /// Distinct from a full [`Self::call`] in intent: used to poll
    /// whether a just-launched project has finished booting its server.
    pub fn ping(&self) -> Result<(), ClientError> {
        match self.call(Method::Ping)? {
            ResponseData::Pong => Ok(()),
            other => Err(ClientError::Unexpected(other)),
        }
    }

    /// Every non-ephemeral entity with its components and fields.
    ///
    /// Kept for callers that want the world outright — tests, and any
    /// one-shot inspection. A client mirroring every frame wants
    /// [`RemoteClient::list_entities_since`] instead, which is what
    /// makes an unchanged scene cost nothing to receive.
    pub fn list_entities(&self) -> Result<Vec<EntitySnapshot>, ClientError> {
        Ok(self.list_entities_since(None)?.entities)
    }

    /// The world, or what changed in it since `since`.
    ///
    /// Pass the `revision` from the previous reply. The server decides
    /// whether it can honour the request and says so in
    /// [`EntityUpdate::full`] — a caller must not assume, because
    /// merging what it thought was a diff would keep entities the
    /// project has deleted.
    pub fn list_entities_since(&self, since: Option<u64>) -> Result<EntityUpdate, ClientError> {
        match self.call(Method::ListEntities { since })? {
            ResponseData::Entities {
                entities,
                removed,
                revision,
                full,
                host,
                scenes,
            } => Ok(EntityUpdate {
                entities,
                removed,
                revision,
                full,
                host,
                scenes,
            }),
            other => Err(ClientError::Unexpected(other)),
        }
    }

    /// Opens an empty unsaved scene on the project and makes it active.
    pub fn new_scene(&self) -> Result<kooch_core::Guid, ClientError> {
        match self.call(Method::NewScene)? {
            ResponseData::SceneOpened { scene } => Ok(scene),
            other => Err(ClientError::Unexpected(other)),
        }
    }

    /// Every registered component type with its field schema.
    pub fn get_schema(&self) -> Result<Vec<ComponentSchema>, ClientError> {
        match self.call(Method::GetSchema)? {
            ResponseData::Schema { components } => Ok(components),
            other => Err(ClientError::Unexpected(other)),
        }
    }

    /// Overwrites one field of one component on one entity.
    pub fn set_field(
        &self,
        entity: EntityId,
        component: &str,
        field: &str,
        value: ReflectValue,
    ) -> Result<(), ClientError> {
        self.expect_ok(Method::SetField {
            entity,
            component: component.to_owned(),
            field: field.to_owned(),
            value,
        })
    }

    /// Adds a default-constructed component to an entity.
    pub fn add_component(&self, entity: EntityId, component: &str) -> Result<(), ClientError> {
        self.expect_ok(Method::AddComponent {
            entity,
            component: component.to_owned(),
        })
    }

    /// Removes a component from an entity.
    pub fn remove_component(&self, entity: EntityId, component: &str) -> Result<(), ClientError> {
        self.expect_ok(Method::RemoveComponent {
            entity,
            component: component.to_owned(),
        })
    }

    /// Spawns a new entity, optionally named; returns its handle.
    ///
    /// `scene` names the scene to author it into (`None` = the active
    /// one) and `parent` what to hang it off. A parent already names the
    /// scene, so `scene` is ignored when one is given.
    pub fn spawn(
        &self,
        name: Option<&str>,
        scene: Option<kooch_core::Guid>,
        parent: Option<EntityId>,
    ) -> Result<EntityId, ClientError> {
        match self.call(Method::Spawn {
            name: name.map(str::to_owned),
            scene,
            parent,
        })? {
            ResponseData::Spawned { entity } => Ok(entity),
            other => Err(ClientError::Unexpected(other)),
        }
    }

    /// Despawns an entity.
    pub fn despawn(&self, entity: EntityId) -> Result<(), ClientError> {
        self.expect_ok(Method::Despawn { entity })
    }

    /// Reparents an entity on the server, or unparents it with `None`.
    ///
    /// Not expressible as `set_field`: `Parent::reflect_set` is read-only
    /// because an entity handle is not a reflectable value.
    pub fn set_parent(
        &self,
        entity: EntityId,
        parent: Option<EntityId>,
    ) -> Result<(), ClientError> {
        self.expect_ok(Method::SetParent { entity, parent })
    }

    /// Persists one open scene to a file on the server's disk.
    ///
    /// `scene` names it; `None` saves the active one. Only that scene's
    /// entities are written — see [`Method::SaveScene`].
    pub fn save_scene(
        &self,
        path: &str,
        scene: Option<kooch_core::Guid>,
    ) -> Result<(), ClientError> {
        self.expect_ok(Method::SaveScene {
            path: path.to_owned(),
            scene,
        })
    }

    /// Writes one entity and its descendants to a scene file — a prefab.
    pub fn save_prefab(&self, entity: EntityId, path: &str) -> Result<(), ClientError> {
        self.expect_ok(Method::SavePrefab {
            entity,
            path: path.to_owned(),
        })
    }

    /// Tells the project an asset file was written, so it stops using the
    /// copy it read first — and learns about the file at all if it is new.
    pub fn reload_asset(&self, path: &str) -> Result<(), ClientError> {
        self.expect_ok(Method::ReloadAsset {
            path: path.to_owned(),
        })
    }

    /// Stamps a prefab file into the live world; returns its root.
    pub fn instantiate_prefab(&self, path: &str) -> Result<EntityId, ClientError> {
        match self.call(Method::InstantiatePrefab {
            path: path.to_owned(),
        })? {
            ResponseData::Spawned { entity } => Ok(entity),
            other => Err(ClientError::Unexpected(other)),
        }
    }

    /// Replaces the server's live ECS with a scene file from its disk.
    /// Starts or stops the project's gameplay systems.
    ///
    /// Stopping restores the world as it stood when play began, which
    /// respawns every entity — treat previously held [`EntityId`]s as
    /// stale afterwards.
    pub fn set_playing(&self, playing: bool) -> Result<(), ClientError> {
        self.call(Method::SetPlaying { playing })?;
        Ok(())
    }

    pub fn load_scene(&self, path: &str) -> Result<(), ClientError> {
        self.expect_ok(Method::LoadScene {
            path: path.to_owned(),
        })
    }

    /// Runs a method whose only success shape is [`ResponseData::Ok`].
    fn expect_ok(&self, method: Method) -> Result<(), ClientError> {
        match self.call(method)? {
            ResponseData::Ok => Ok(()),
            other => Err(ClientError::Unexpected(other)),
        }
    }

    /// Sends one request and returns the server's result, mapping a
    /// typed [`RemoteError`] to [`ClientError::Remote`].
    pub fn call(&self, method: Method) -> Result<ResponseData, ClientError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = Request { id, method };
        let body =
            serde_json::to_string(&request).map_err(|e| ClientError::Decode(e.to_string()))?;

        let started = Instant::now();
        let raw = self.round_trip(&body)?;
        let transport = started.elapsed();

        let decoding = Instant::now();
        // The reply is quoted into the error on failure. Without it a
        // decode error names the missing field and not the text that
        // lacked it, which says nothing about whether the reply was
        // truncated, framed wrong, or simply not what was expected.
        let decoded: Result<Response, _> = serde_json::from_str(&raw).map_err(|e| {
            let head: String = raw.chars().take(200).collect();
            ClientError::Decode(format!(
                "{e} — reply was {} bytes, starting: {head:?}",
                raw.len()
            ))
        });
        // Recorded before the `?`, so a payload that is expensive to
        // parse *and* malformed still reports what it cost.
        self.last_call
            .store(transport, decoding.elapsed(), raw.len());
        let response = decoded?;

        match response.payload {
            ResponsePayload::Result(data) => Ok(data),
            ResponsePayload::Error(err) => Err(ClientError::Remote(err)),
        }
    }

    /// Connects, writes one JSON line, and reads the reply line.
    fn round_trip(&self, body: &str) -> Result<String, ClientError> {
        let name = self
            .name
            .as_str()
            .to_ns_name::<GenericNamespaced>()
            .map_err(|e| ClientError::Decode(format!("invalid socket name: {e}")))?;
        let stream = Stream::connect(name)?;
        let mut conn = BufReader::new(stream);

        conn.get_mut().write_all(body.as_bytes())?;
        conn.get_mut().write_all(b"\n")?;

        let mut reply = String::new();
        conn.read_line(&mut reply)?;
        if reply.is_empty() {
            return Err(ClientError::Decode(
                "server closed without replying".to_owned(),
            ));
        }
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;

/// One reply to [`RemoteClient::list_entities_since`].
///
/// `full` is the field that matters: it decides whether the caller
/// replaces its mirror or merges into it, and merging a full reply
/// would silently keep whatever the reply left out.
#[derive(Debug, Clone)]
pub struct EntityUpdate {
    /// The whole world when `full`, otherwise only what changed.
    pub entities: Vec<EntitySnapshot>,
    /// Entities that no longer exist. Empty in a full reply — absence
    /// says it there.
    pub removed: Vec<crate::protocol::EntityId>,
    /// Pass back as `since` on the next call.
    pub revision: u64,
    pub full: bool,
    /// What the host's frame cost, if it reported one.
    pub host: Option<crate::protocol::HostMetrics>,
    /// The scenes the project has open, or `None` if it did not say.
    ///
    /// Not diffed: it arrives whole or not at all, so a caller replaces
    /// its list rather than merging into one.
    pub scenes: Option<Vec<crate::protocol::SceneEntry>>,
}
