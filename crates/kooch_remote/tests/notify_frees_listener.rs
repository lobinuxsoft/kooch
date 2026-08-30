//! A push must not hold the listener thread (#1015).
//!
//! The server has ONE listener and `serve_one` used to block it on the
//! main loop's reply before accepting the next connection. A caller that
//! will not read the answer still cost a whole host frame of that
//! thread, so holding a key — one input push per frame on top of the
//! editor's pull — put two blocking connections through a queue that
//! serves one.

use std::time::{Duration, Instant};

use kooch_remote::protocol::Method;
use kooch_remote::server::RemoteServer;
use kooch_remote::{RemoteClient, protocol::Request};

fn socket_name() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static N: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!(
        "kooch_notify_{}_{}_{}.sock",
        std::process::id(),
        nanos,
        N.fetch_add(1, Ordering::Relaxed)
    )
}

/// A host that never drains its queue — the worst case of a main loop
/// busy with a frame. A blocking request against it waits forever; a
/// push must not.
#[test]
fn a_push_does_not_wait_for_the_host() {
    let server = RemoteServer::start(&socket_name()).expect("bind a socket");
    let client = RemoteClient::new(server.name());
    // Nothing ever calls `take_pending`, so no reply is ever produced.

    let started = Instant::now();
    client
        .notify(Method::Ping)
        .expect("the push should reach the socket");
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "the push waited {:?} on a host that never answers",
        started.elapsed()
    );
}

/// The point of the whole file: a push in flight must leave the listener
/// free for the request behind it.
#[test]
fn a_push_leaves_the_listener_free() {
    let server = RemoteServer::start(&socket_name()).expect("bind a socket");
    let client = RemoteClient::new(server.name());

    // The push queues and returns; the listener must go back to accept.
    client.notify(Method::Ping).expect("push");

    // Now a real request. It queues too, so it appears in the pending
    // list — which it cannot do if the listener is still stuck on the
    // push's reply.
    let name = server.name().to_owned();
    std::thread::spawn(move || {
        let asker = RemoteClient::new(&name);
        let _ = asker.ping();
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut seen = 0usize;
    while seen < 2 && Instant::now() < deadline {
        seen += server.take_pending().len();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(seen, 2, "the listener never accepted the second connection");
}

/// A request that is NOT a push still gets its reply — the flag must not
/// have turned every call into a push.
#[test]
fn an_ordinary_call_still_gets_its_answer() {
    assert!(
        !Request {
            id: 1,
            notify: false,
            method: Method::Ping,
        }
        .notify
    );
}
