//! Remote editor protocol — drive a running project's ECS from outside.
//!
//! A project built with the engine knows its own component types; the
//! standalone editor never can, because Rust resolves types at compile
//! time. Rather than load the project's code into the editor (there is
//! no stable Rust ABI for that), the editor stays a thin client and the
//! **project process owns the ECS** and answers requests over HTTP.
//!
//! This mirrors Bevy's Remote Protocol: the running app is the server,
//! an external tool is the client, and every payload is keyed by
//! fully-qualified type name so neither side needs the other's
//! [`std::any::TypeId`].
//!
//! ## Threading
//!
//! The engine loop is single-threaded and touches no async runtime, and
//! this crate keeps it that way. The socket listener blocks on a
//! **dedicated thread**; each request is forwarded to the main loop over
//! a channel
//! and answered there, so the ECS is only ever touched from the main
//! thread. See [`server`]. Nothing here makes the engine async or
//! multi-threaded in its own logic — the server is one side thread with
//! a message queue.
//!
//! ## Layers
//!
//! - [`protocol`] — the wire types: requests, responses, errors, and the
//!   by-name snapshots of ECS state. Pure data, no I/O.
//! - [`server`] — the HTTP listener thread and the main-thread bridge.
//! - [`handlers`] — executes a [`protocol::Request`] against the live
//!   [`Resources`](kooch_core::resource::Resources).
//! - [`plugin`] — [`RemotePlugin`], which starts the server and registers
//!   the drain system.

pub mod client;
pub mod handlers;
pub mod plugin;
/// Re-exported for extension authors: a handler's payload and result are
/// `serde_json::Value`, and the crate that writes one should not have to
/// depend on serde_json to say so.
pub use serde_json;

pub mod extensions;
pub mod protocol;
pub mod server;

pub use client::{CallStats, ClientError, RemoteClient};
pub use plugin::RemotePlugin;
pub use protocol::{Request, Response};
pub use server::{DEFAULT_NAME, NAME_ENV, RemoteServer};
