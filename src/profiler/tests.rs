use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use kooch_core::app::App;
use kooch_core::runner::run_for_frames;

use super::ProfilingPlugin;

/// Counts the frames puffin actually publishes while an app runs.
///
/// Takes a port nobody will connect to: the boundary is what is under
/// test, and the server is the part that would fight another test for a
/// socket.
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
/// silently becomes a graph of half-frames.
#[test]
fn a_frame_publishes_once() {
    // The first `new_frame` closes the frame that was open before the app
    // started, which carries no scopes of ours and which puffin drops.
    assert_eq!(published_frames(4), 4);
}
