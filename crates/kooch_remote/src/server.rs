//! Listener thread and its bridge to the main loop, the only thread touching the ECS.
//! A per-session local socket, never a TCP port a web page could reach (#647); one JSON object per
//! line.

use std::io::{BufRead, BufReader, Write};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions, Stream, ToNsName, prelude::*,
};

use kooch_core::frame_pacing::FrameWaker;

use crate::protocol::{Request, Response};

/// Environment variable carrying the socket name to a launched project; without it [`DEFAULT_NAME`]
/// keeps `cargo run -- --remote` working.
pub const NAME_ENV: &str = "KOOCH_REMOTE_SOCKET";

/// Socket name used when nothing set [`NAME_ENV`].
pub const DEFAULT_NAME: &str = "kooch_remote_default.sock";

/// One in-flight request: the decoded call plus the channel the listener
/// thread waits on for its answer.
pub struct PendingRequest {
    pub request: Request,
    pub reply: Sender<Response>,
}

/// Main-loop handle to the running server: the request queue's receiver, whose drop ends the
/// listener thread at its next hand-off.
pub struct RemoteServer {
    /// Requests decoded by the listener, awaiting execution — behind an always-uncontended `Mutex`,
    /// since a resource must be `Sync`.
    incoming: Mutex<Receiver<PendingRequest>>,
    /// Kept alive so the listener thread runs for the server's lifetime.
    _listener: JoinHandle<()>,
    /// The bound name, for logging and tests.
    name: String,
}

impl RemoteServer {
    /// Binds `name` and spawns the listener with no way to wake a sleeping main loop — for tests;
    /// projects use [`Self::start_waking`]. Errors when the name is taken.
    pub fn start(name: &str) -> Result<Self, String> {
        Self::start_waking(name, FrameWaker::default())
    }

    /// Binds `name` and spawns the listener, waking the main loop per queued request — once it may
    /// sleep (#656), a request would otherwise wait for an unrelated frame.
    pub fn start_waking(name: &str, waker: FrameWaker) -> Result<Self, String> {
        let ns_name = name
            .to_ns_name::<GenericNamespaced>()
            .map_err(|e| format!("invalid socket name {name}: {e}"))?;
        let listener = ListenerOptions::new()
            .name(ns_name)
            .create_sync()
            .map_err(|e| format!("failed to bind remote socket {name}: {e}"))?;

        let (tx, rx) = channel::<PendingRequest>();
        let handle = std::thread::Builder::new()
            .name("kooch_remote".into())
            .spawn(move || listen(listener, tx, waker))
            .map_err(|e| format!("failed to spawn remote server thread: {e}"))?;

        tracing::info!(socket = name, "remote editor server listening");
        Ok(Self {
            incoming: Mutex::new(rx),
            _listener: handle,
            name: name.to_owned(),
        })
    }

    /// Binds the name in [`NAME_ENV`], or [`DEFAULT_NAME`] if unset.
    pub fn start_from_env(waker: FrameWaker) -> Result<Self, String> {
        let name = std::env::var(NAME_ENV).unwrap_or_else(|_| DEFAULT_NAME.to_owned());
        Self::start_waking(&name, waker)
    }

    /// The bound socket name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Drains every queued request without blocking, once per tick; each must be answered through
    /// `reply` to free the listener.
    pub fn take_pending(&self) -> Vec<PendingRequest> {
        match self.incoming.lock() {
            Ok(rx) => rx.try_iter().collect(),
            // The listener thread never locks this Mutex, so it cannot
            // poison it; treat a poisoned lock as "no requests" rather
            // than panicking the main loop.
            Err(_) => Vec::new(),
        }
    }
}

/// Listener thread body: accept, decode, hand off, wait for the reply,
/// and write it back. Runs until the socket errors or the main-thread
/// receiver is dropped.
fn listen(
    listener: interprocess::local_socket::Listener,
    tx: Sender<PendingRequest>,
    waker: FrameWaker,
) {
    for conn in listener.incoming() {
        let conn = match conn {
            Ok(conn) => conn,
            Err(e) => {
                tracing::debug!("remote: accept failed: {e}");
                continue;
            }
        };
        if !serve_one(conn, &tx, &waker) {
            break;
        }
    }
}

/// Encodes a response, falling back to an encodable complaint carrying the serialiser's message —
/// `{}` used to break the client with an unrelated error.
fn encode(response: &Response) -> String {
    match serde_json::to_string(response) {
        Ok(json) => json,
        Err(e) => {
            tracing::error!(
                target: "kooch_remote",
                id = response.id,
                error = %e,
                "a response could not be encoded",
            );
            let complaint = Response::err(
                response.id,
                crate::protocol::RemoteError::Unavailable {
                    detail: format!("the reply could not be encoded: {e}"),
                },
            );
            // Strings and a number: nothing here can fail. Spelled out
            // rather than unwrapped, because this path exists precisely
            // for the case where a serialiser did fail.
            serde_json::to_string(&complaint).unwrap_or_else(|_| {
                format!(
                    r#"{{"id":{},"error":{{"code":"unavailable","detail":"the reply could not be encoded"}}}}"#,
                    response.id,
                )
            })
        }
    }
}

/// Handles one connection. Returns `false` when the main loop is gone and
/// the listener should stop.
fn serve_one(conn: Stream, tx: &Sender<PendingRequest>, waker: &FrameWaker) -> bool {
    let mut reader = BufReader::new(conn);
    let mut line = String::new();
    if let Err(e) = reader.read_line(&mut line) {
        tracing::debug!("remote: failed to read request: {e}");
        return true;
    }

    let response = match serde_json::from_str::<Request>(&line) {
        Ok(request) => {
            // 🔴 Read before the move: a push has no reader, so blocking on the main loop's answer
            // only delays the next caller (#1015).
            let quiet = request.notify;
            let (reply_tx, reply_rx) = channel::<Response>();
            let pending = PendingRequest {
                request,
                reply: reply_tx,
            };
            // A send error means the main loop dropped the server; stop.
            if tx.send(pending).is_err() {
                return false;
            }
            // Wake after queuing, never before, or the loop could find nothing and sleep as the
            // request lands.
            waker.wake();
            if quiet {
                // Queued and done: the reply lands on a dropped channel `handle` tolerates, freeing
                // the listener for the next pull.
                return true;
            }
            // Block until the main thread executes and answers. If the
            // reply channel drops, the caller gets nothing and times out.
            match reply_rx.recv() {
                Ok(response) => response,
                Err(_) => return false,
            }
        }
        Err(e) => Response::err(
            0,
            crate::protocol::RemoteError::BadRequest {
                detail: e.to_string(),
            },
        ),
    };

    let mut json = encode(&response);
    json.push('\n');
    if let Err(e) = reader.get_mut().write_all(json.as_bytes()) {
        tracing::debug!("remote: failed to write response: {e}");
    }
    true
}
