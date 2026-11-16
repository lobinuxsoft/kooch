//! A long capture must not throw away its own scope names (#785).

use std::sync::{Arc, Mutex};

/// Frames to record — past `FrameView`'s `max_recent`, which is 1000.
const FRAMES: usize = 1100;

/// How many of those are cheap.
const CHEAP: usize = 300;

/// The scope whose name has to survive the round trip.
const SCOPE: &str = "kooch_capture_names_probe";

/// Records `FRAMES` frames into puffin's default (bounded) view, the way a client receives them.
fn record() -> puffin::FrameView {
    let view = Arc::new(Mutex::new(puffin::FrameView::default()));

    let sink_view = Arc::clone(&view);
    let sink = puffin::GlobalProfiler::lock().add_sink(Box::new(move |frame| {
        sink_view.lock().unwrap().add_frame(frame)
    }));

    puffin::set_scopes_on(true);
    // What `puffin_http::Server` does for every client that connects: hand it the whole scope
    // collection. It arrives as the delta of the next frame, which is precisely the frame at risk
    // of being evicted.
    puffin::GlobalProfiler::lock().emit_scope_snapshot();

    for frame in 0..FRAMES {
        {
            puffin::profile_scope!(SCOPE);
            if frame >= CHEAP {
                // Enough to sort above the early frames without making
                // the test slow: 800 sleeps of 50 µs is well under a
                // tenth of a second in total.
                std::thread::sleep(std::time::Duration::from_micros(50));
            }
        }
        puffin::GlobalProfiler::lock().new_frame();
    }

    puffin::set_scopes_on(false);
    puffin::GlobalProfiler::lock().remove_sink(sink);

    let view = std::mem::take(&mut *view.lock().unwrap());
    view
}

/// Whether the name survives being written to a `.puffin` and read back.
fn survives_round_trip(view: &puffin::FrameView, ours: bool) -> bool {
    let mut bytes = Vec::new();
    if ours {
        kooch_editor_core::write_capture(view, &mut bytes).expect("write the capture");
    } else {
        view.write(&mut bytes).expect("write the capture");
    }
    let read = puffin::FrameView::read(&mut bytes.as_slice()).expect("read the capture back");
    read.scope_collection().fetch_by_name(SCOPE).is_some()
}

/// Both cases, in one test, on purpose.
#[test]
fn a_long_capture_keeps_its_names() {
    let view = record();
    // The cause, pinned first. If puffin ever serialises the collection itself, this fails and
    // says `write_capture` can go — nothing else would.
    assert!(
        !survives_round_trip(&view, false),
        "puffin's own write kept the names across {FRAMES} frames; re-check whether \
         `write_capture` is still needed before deleting it.",
    );

    assert!(
        survives_round_trip(&view, true),
        "a {FRAMES}-frame capture lost the name of {SCOPE}. The view evicted the frame that \
         carried it, and `write_capture` must write the view's whole collection instead.",
    );
}
