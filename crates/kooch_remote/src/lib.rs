//! Remote editor protocol: the project process owns the ECS and the standalone editor drives it
//! over a local socket, keyed by type name since the editor cannot know the project's types.
//! A blocking listener thread hands requests to the main loop, the only thread touching the ECS.

pub mod client;
pub mod handlers;
pub mod plugin;
/// Re-exported for extension authors: a handler's payload and result are
/// `serde_json::Value`, and the crate that writes one should not have to
/// depend on serde_json to say so.
pub use serde_json;

pub mod extensions;
pub mod moved_cache;
pub mod protocol;
pub mod server;
pub mod snapshot_cache;

pub use client::{CallStats, ClientError, MovedUpdate, RemoteClient};
pub use plugin::RemotePlugin;
pub use protocol::{Request, Response};
pub use server::{DEFAULT_NAME, NAME_ENV, RemoteServer};
