use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use kooch_core::app::App;
use kooch_core::runner::run_for_frames;

use super::ProfilingPlugin;

/// Taken by every test in this file.
///
/// 🔴 puffin's profiler is a process-wide singleton and so is
/// `set_scopes_on`, so two of these running at once would each see the
/// other's frames — and cargo's test harness is threaded. The suite is
/// tiny; serialising it costs nothing and a flaky profiler test teaches
/// people to ignore it.
static PUFFIN: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Counts the frames puffin actually publishes while an app runs.
///
/// Binds a port nothing will connect to: the boundary is what is under
/// test here, not the transport.
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

/// One frame in, one frame out.
///
/// Fails when the boundary system is missing (puffin grows a single
/// unbounded frame and publishes nothing) and fails again if a second
/// boundary is ever added anywhere in a frame, which is how a flamegraph
/// silently becomes a graph of half-frames. Both directions verified by
/// breaking them.
#[test]
fn a_frame_publishes_once() {
    let _guard = PUFFIN
        .lock()
        .expect("the puffin test lock is never poisoned");
    assert_eq!(published_frames(4), 4);
}

/// The whole wire, without the editor: a game serves, a viewer connects,
/// and what arrives carries **names**.
///
/// The names are the point. A frame whose scopes read `scope#ScopeId(67)`
/// looks like a successful capture and is worth nothing, and the reason
/// it works here is entirely inside `puffin_http`: the server keeps its
/// own `ScopeCollection` and re-sends all of it to each client that
/// arrives. Nothing in this crate arranges that, which is exactly why it
/// is worth a test — an upgrade that changed it would otherwise be found
/// by squinting at a flamegraph.
#[test]
fn a_viewer_receives_named_frames() {
    let _guard = PUFFIN
        .lock()
        .expect("the puffin test lock is never poisoned");

    // A fixed port, because `puffin_http::Server` does not report the one
    // the OS picked. High and unregistered; a machine already serving
    // here fails this test rather than reporting a false pass.
    const ADDR: &str = "127.0.0.1:18585";

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
