use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use kooch_core::app::App;
use kooch_core::runner::run_for_frames;

use super::ProfilingPlugin;

/// Serialises this file: puffin's profiler and `set_scopes_on` are process-wide, and the harness is
/// threaded.
static PUFFIN: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Counts frames puffin publishes; the port is never connected, since the boundary is under test.
fn published_frames(frames_to_run: u32) -> usize {
    let count = Arc::new(AtomicUsize::new(0));
    let sink_count = Arc::clone(&count);
    let sink = puffin::GlobalProfiler::lock().add_sink(Box::new(move |_frame| {
        sink_count.fetch_add(1, Ordering::SeqCst);
    }));

    let mut app = App::new();
    app.add_plugin(ProfilingPlugin {
        bind_addr: "127.0.0.1:0".to_string(),
    });
    run_for_frames(app, frames_to_run);

    puffin::GlobalProfiler::lock().remove_sink(sink);
    count.load(Ordering::SeqCst)
}

/// One frame in, one out: fails without a boundary and with a second one. Both directions verified
/// by breaking them.
#[test]
fn a_frame_publishes_once() {
    let _guard = PUFFIN
        .lock()
        .expect("the puffin test lock is never poisoned");
    assert_eq!(published_frames(4), 4);
}

/// The whole wire: a game serves, a viewer connects, and scopes arrive with **names** —
/// `puffin_http` re-sends its collection per client, and an upgrade changing that would otherwise
/// go unseen.
#[test]
fn a_viewer_receives_named_frames() {
    let _guard = PUFFIN
        .lock()
        .expect("the puffin test lock is never poisoned");

    // A fixed port, because `puffin_http::Server` does not report the one
    // the OS picked. High and unregistered; a machine already serving
    // here fails this test rather than reporting a false pass.
    const ADDR: &str = "127.0.0.1:18585";

    // 🔴 Scopes run before the server exists on purpose, or the late-server path is tested only by
    // test order.
    puffin::set_scopes_on(true);
    let mut warmup = App::new();
    warmup.schedule.run_frame_stages(&mut warmup.resources);
    // The boundary is what drains `new_scopes`. Without it the names
    // would still be queued and the next frame would carry them anyway,
    // which is why running frames alone does not reproduce this.
    puffin::GlobalProfiler::lock().new_frame();

    let mut app = App::new();
    app.add_plugin(ProfilingPlugin {
        bind_addr: ADDR.to_string(),
    });
    let client = puffin_http::Client::new(ADDR.to_string());

    app.schedule.run_startup(&mut app.resources);

    // The client retries on a one-second cadence and the server sends
    // nothing until someone is listening, so this is a wait, not a
    // handshake. Frames keep being produced throughout.
    const WAIT: std::time::Duration = std::time::Duration::from_secs(15);
    let deadline = std::time::Instant::now() + WAIT;
    let named = loop {
        app.schedule.run_frame_stages(&mut app.resources);

        let received = client.frame_view();
        let has_frame = received.all_uniq().next().is_some();
        // `Update` is a stage of every frame, named by `run_staged!` in
        // the schedule. If the transport works and this is missing, the
        // capture arrived without its scope details.
        let named = received
            .scope_collection()
            .fetch_by_name("Update")
            .is_some();
        drop(received);

        if has_frame && named {
            break true;
        }
        if std::time::Instant::now() > deadline {
            break false;
        }
        std::thread::sleep(std::time::Duration::from_millis(16));
    };

    assert!(
        named,
        "no named frame reached the client in {WAIT:?} — connected: {}",
        client.connected()
    );
}
