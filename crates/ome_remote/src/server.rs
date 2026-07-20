//! HTTP listener thread and its bridge to the main loop.
//!
//! [`tiny_http`] blocks on `recv()` on a dedicated thread, so nothing
//! here runs on the engine's critical path and no async runtime is
//! pulled in. Each request is decoded, handed to the main thread over a
//! channel, and answered there — the ECS is only ever touched from the
//! main thread. The listener thread parks on a per-request reply channel
//! until the main loop drains the queue.
//!
//! The main loop drives the drain via
//! [`RemoteServer::take_pending`], which the plugin's system calls once
//! per tick.

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use crate::protocol::{Request, Response};

/// Default TCP port the server binds. Chosen out of the ephemeral range
/// and distinct from Bevy Remote's 15702 to avoid clashing on a machine
/// running both.
pub const DEFAULT_PORT: u16 = 15703;

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
    /// A [`Resources`](ome_core::resource::Resources) entry must be
    /// `Sync`, which [`Receiver`] is not, so it lives behind a `Mutex`.
    /// The lock is always uncontended — only the main thread drains it.
    incoming: Mutex<Receiver<PendingRequest>>,
    /// Kept alive so the listener thread runs for the server's lifetime.
    _listener: JoinHandle<()>,
    /// The bound port, for logging and tests.
    port: u16,
}

impl RemoteServer {
    /// Binds `port` and spawns the listener thread.
    ///
    /// Returns an error string if the port cannot be bound (already in
    /// use, or insufficient permission).
    pub fn start(port: u16) -> Result<Self, String> {
        let server = tiny_http::Server::http(("127.0.0.1", port))
            .map_err(|e| format!("failed to bind remote server on port {port}: {e}"))?;
        let (tx, rx) = channel::<PendingRequest>();

        let listener = std::thread::Builder::new()
            .name("ome_remote".into())
            .spawn(move || listen(server, tx))
            .map_err(|e| format!("failed to spawn remote server thread: {e}"))?;

        tracing::info!(port, "remote editor server listening");
        Ok(Self {
            incoming: Mutex::new(rx),
            _listener: listener,
            port,
        })
    }

    /// The bound port.
    pub fn port(&self) -> u16 {
        self.port
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
fn listen(server: tiny_http::Server, tx: Sender<PendingRequest>) {
    for mut http_request in server.incoming_requests() {
        let mut body = String::new();
        if std::io::Read::read_to_string(http_request.as_reader(), &mut body).is_err() {
            let _ = http_request.respond(tiny_http::Response::empty(400));
            continue;
        }

        let response = match serde_json::from_str::<Request>(&body) {
            Ok(request) => {
                let (reply_tx, reply_rx) = channel::<Response>();
                let pending = PendingRequest {
                    request,
                    reply: reply_tx,
                };
                // A send error means the main loop dropped the server;
                // stop the thread.
                if tx.send(pending).is_err() {
                    break;
                }
                // Block until the main thread executes and answers. If
                // the reply channel drops (server torn down mid-request),
                // fall through to a 503.
                reply_rx.recv().ok()
            }
            Err(e) => {
                let body = crate::protocol::Response::err(
                    0,
                    crate::protocol::RemoteError::BadRequest {
                        detail: e.to_string(),
                    },
                );
                Some(body)
            }
        };

        let sent = match response {
            Some(response) => {
                let json = serde_json::to_string(&response).unwrap_or_else(|_| "{}".into());
                http_request.respond(json_response(json, 200))
            }
            None => http_request.respond(tiny_http::Response::empty(503)),
        };
        if let Err(e) = sent {
            tracing::debug!("remote: failed to write response: {e}");
        }
    }
}

/// Wraps a JSON string in an HTTP response with the right content type.
fn json_response(body: String, status: u16) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
        .expect("static header is valid");
    tiny_http::Response::from_string(body)
        .with_status_code(status)
        .with_header(header)
}
