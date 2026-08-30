//! Listener thread and its bridge to the main loop.
//!
//! The listener blocks on a dedicated thread, so nothing here runs on the
//! engine's critical path and no async runtime is pulled in. Each request
//! is decoded, handed to the main thread over a channel, and answered
//! there — the ECS is only ever touched from the main thread. The
//! listener parks on a per-request reply channel until the main loop
//! drains the queue.
//!
//! # Why a local socket and not a TCP port
//!
//! The editor and the project it launched are on the same machine, always.
//! A TCP port is reachable by anything else on that machine — including a
//! web page, which can `fetch()` at a loopback address and, with a
//! `text/plain` body, do it without a CORS preflight. The server would
//! then run whatever it was sent. Against `SaveScene`, which writes to any
//! path, that is code execution from visiting a page (#647).
//!
//! A local socket is not addressable that way. On Unix it is a domain
//! socket, on Windows a named pipe; a browser can reach neither. The
//! vector disappears rather than being filtered for.
//!
//! # Why the name is unique per session
//!
//! The old port was the constant 15703, so an orphaned project — one that
//! outlived a crashed editor — kept holding it, and the next editor
//! connected to *that* instead of the process it had just launched, then
//! mirrored and edited a dead session's world in silence. A name minted
//! per session cannot be answered by yesterday's process.
//!
//! # Framing
//!
//! One JSON object per line, both ways. HTTP bought nothing here: there is
//! no routing, no content negotiation and no caching — only a request and
//! a reply on a channel that already guarantees order.

use std::io::{BufRead, BufReader, Write};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions, Stream, ToNsName, prelude::*,
};

use kooch_core::frame_pacing::FrameWaker;

use crate::protocol::{Request, Response};

/// Environment variable carrying the socket name to a launched project.
///
/// The editor mints the name and passes it down; the project reads it
/// here. A project started by hand with no name set falls back to
/// [`DEFAULT_NAME`], which is what makes `cargo run -- --remote` still
/// work on its own.
pub const NAME_ENV: &str = "KOOCH_REMOTE_SOCKET";

/// Socket name used when nothing set [`NAME_ENV`].
pub const DEFAULT_NAME: &str = "kooch_remote_default.sock";

/// One in-flight request: the decoded call plus the channel the listener
/// thread waits on for its answer.
pub struct PendingRequest {
    pub request: Request,
    pub reply: Sender<Response>,
}

/// Main-loop handle to the running server.
///
/// Holds the receiving end of the request queue. The listener thread and
/// its socket live behind [`Self::_listener`]; dropping this resource
/// drops the receiver, and the listener thread exits the next time it
/// tries to hand off a request.
pub struct RemoteServer {
    /// Requests decoded by the listener thread, awaiting execution.
    ///
    /// A [`Resources`](kooch_core::resource::Resources) entry must be
    /// `Sync`, which [`Receiver`] is not, so it lives behind a `Mutex`.
    /// The lock is always uncontended — only the main thread drains it.
    incoming: Mutex<Receiver<PendingRequest>>,
    /// Kept alive so the listener thread runs for the server's lifetime.
    _listener: JoinHandle<()>,
    /// The bound name, for logging and tests.
    name: String,
}

impl RemoteServer {
    /// Binds `name` and spawns the listener thread, with no way to wake
    /// a sleeping main loop.
    ///
    /// Fine for a test, which drives the frame itself. A real project
    /// wants [`Self::start_waking`] — see it for why.
    ///
    /// Returns an error string if the name cannot be bound — most often
    /// because another process already holds it.
    pub fn start(name: &str) -> Result<Self, String> {
        Self::start_waking(name, FrameWaker::default())
    }

    /// Binds `name` and spawns the listener thread, waking the main loop
    /// whenever a request is queued.
    ///
    /// The listener parks on a reply that only the main thread can
    /// produce. Once that thread is allowed to sleep between frames
    /// (#656), a request arriving while it sleeps would be answered
    /// only when something else happened to wake it — from the editor's
    /// side, an indefinite hang on a project that is running perfectly
    /// well. The wake is what keeps "asleep" from meaning "unreachable".
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

    /// Drains every request decoded since the last call, without blocking.
    ///
    /// The main loop calls this once per tick; each returned request must
    /// be answered through its `reply` channel to unblock the listener.
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

/// Encodes a response, falling back to an encodable complaint about why
/// it could not be encoded.
///
/// This used to be `unwrap_or_else(|_| "{}")`. The client then failed
/// decoding `{}` — a different error, in a different process, naming
/// nothing — and its mirror stopped updating for good. A snapshot that
/// cannot be described has to say so, in the reply, where the caller is
/// already looking.
///
/// The fallback carries the serialiser's own message: whatever refused to
/// encode says why, and that sentence is the whole diagnosis.
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
            // 🔴 Read BEFORE the move, and acted on after the queue: a
            // push has no reader, so blocking this thread on the main
            // loop's answer buys nothing and costs the next caller a
            // whole host frame (#1015).
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
            // Queued, so wake whoever drains the queue. Ordering matters:
            // waking before the send could have the loop run a frame,
            // find nothing, and go back to sleep just as the request
            // lands — a wake for the previous request, wasted on this one.
            waker.wake();
            if quiet {
                // Queued and done. The main loop still executes it — the
                // reply simply lands on a dropped channel, which `handle`
                // already tolerates. What is given up is an answer nobody
                // was going to read; what is bought is the listener being
                // free to accept the pull that comes in behind it.
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
