//! Blocking HTTP client for the remote editor protocol.
//!
//! The editor calls this to drive a project's running ECS. It is the
//! mirror of [`server`](crate::server) and speaks the same
//! [`protocol`](crate::protocol) types, so a request built here
//! deserializes there and back with no schema drift.
//!
//! The transport is a bare `std::net::TcpStream` per request — one
//! `POST`, one JSON body, `Connection: close`, read to EOF. No HTTP
//! client crate and no async runtime: the editor calls from its own
//! frame loop and a tooling client polling a loopback socket does not
//! need connection pooling. If that ever bites, the transport is behind
//! [`RemoteClient::call`] and swappable without touching callers.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ome_ecs::reflect::ReflectValue;

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

/// Connection details for a running project's remote server.
///
/// Cheap to clone and hold; each call opens its own short-lived socket,
/// so a [`RemoteClient`] is just an address plus a request-id counter.
pub struct RemoteClient {
    host: String,
    port: u16,
    /// Monotonic request ids, so a reply can be matched to its call.
    next_id: AtomicU64,
    /// Per-request socket timeout.
    timeout: Duration,
}

impl RemoteClient {
    /// A client for the loopback server on `port`.
    pub fn new(port: u16) -> Self {
        Self {
            host: "127.0.0.1".to_owned(),
            port,
            next_id: AtomicU64::new(1),
            timeout: Duration::from_secs(2),
        }
    }

    /// Overrides the per-request socket timeout (default 2s).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
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
    pub fn list_entities(&self) -> Result<Vec<EntitySnapshot>, ClientError> {
        match self.call(Method::ListEntities)? {
            ResponseData::Entities { entities } => Ok(entities),
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
    pub fn spawn(&self, name: Option<&str>) -> Result<EntityId, ClientError> {
        match self.call(Method::Spawn {
            name: name.map(str::to_owned),
        })? {
            ResponseData::Spawned { entity } => Ok(entity),
            other => Err(ClientError::Unexpected(other)),
        }
    }

    /// Despawns an entity.
    pub fn despawn(&self, entity: EntityId) -> Result<(), ClientError> {
        self.expect_ok(Method::Despawn { entity })
    }

    /// Persists the server's live ECS to a scene file on its disk.
    pub fn save_scene(&self, path: &str) -> Result<(), ClientError> {
        self.expect_ok(Method::SaveScene {
            path: path.to_owned(),
        })
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

        let raw = self.round_trip(&body)?;
        let response: Response =
            serde_json::from_str(&raw).map_err(|e| ClientError::Decode(e.to_string()))?;

        match response.payload {
            ResponsePayload::Result(data) => Ok(data),
            ResponsePayload::Error(err) => Err(ClientError::Remote(err)),
        }
    }

    /// Opens a socket, POSTs `body`, and returns the response body.
    fn round_trip(&self, body: &str) -> Result<String, ClientError> {
        let mut stream = TcpStream::connect((self.host.as_str(), self.port))?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;

        let request = format!(
            "POST / HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.host,
            body.len(),
            body,
        );
        stream.write_all(request.as_bytes())?;

        let mut raw = String::new();
        stream.read_to_string(&mut raw)?;
        split_body(&raw).ok_or_else(|| ClientError::Decode("no HTTP body in response".to_owned()))
    }
}

/// Returns the body that follows the first blank line of an HTTP
/// response, i.e. everything after the `\r\n\r\n` header terminator.
fn split_body(raw: &str) -> Option<String> {
    raw.split_once("\r\n\r\n").map(|(_, body)| body.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_body_takes_everything_after_the_blank_line() {
        let raw = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}";
        assert_eq!(split_body(raw).as_deref(), Some("{}"));
    }

    #[test]
    fn split_body_is_none_without_a_header_terminator() {
        assert_eq!(split_body("garbage without terminator"), None);
    }
}
