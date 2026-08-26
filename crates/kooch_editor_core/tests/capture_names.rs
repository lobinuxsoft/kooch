//! A long capture must not throw away its own scope names (#785).
//!
//! # Why this needs a test at all
//!
//! The failure is invisible to everything that normally catches things.
//! The capture is well-formed, every duration in it is correct, the file
//! opens, the tool prints a full tree — and every row in that tree reads
//! `scope#ScopeId(81)`. Nothing errors, nothing panics, nothing is
//! missing. It cost a session's worth of reading a capture by guessing
//! at magnitudes before the cause was found.
//!
//! And it is **intermittent**: 846 frames came back named, 1022 the same
//! evening did not, but the length is not the rule — see [`CHEAP`] for
//! what actually decides it. A capture that reads fine is not evidence
//! the problem is gone, which is exactly why this is a test and not a
//! habit of capturing shorter.

use std::sync::{Arc, Mutex};

/// Frames to record — past `FrameView`'s `max_recent`, which is 1000.
const FRAMES: usize = 1100;

/// How many of those are cheap.
///
/// 🔴 Going past `max_recent` is not on its own enough to lose the
/// names, and assuming it was is how the first version of this test
/// passed while asserting the opposite. `FrameView` keeps **two** sets:
/// the last `max_recent` frames *and* the slowest `max_slow` (256), and
/// `all_uniq` writes out the union. A synthetic capture's first frame is
/// its slowest — cold caches, first touch of every allocation — so the
/// frame carrying the names was being retained by the second net no
/// matter how many frames came after it.
///
/// So the cheap frames come first and are outnumbered by `FRAMES -
/// CHEAP` = 800 slower ones, which is comfortably more than the 256
/// slots in that second net. Only then is the carrier genuinely evicted.
const CHEAP: usize = 300;

/// The scope whose name has to survive the round trip.
const SCOPE: &str = "kooch_capture_names_probe";

/// Records `FRAMES` frames into a view, the way a client receives them.
///
/// `keep_all` chooses whether the view is the one this crate configures
/// or puffin's default, which is the whole subject of the test.
fn record(keep_all: bool) -> puffin::FrameView {
    let view = Arc::new(Mutex::new(puffin::FrameView::default()));
    if keep_all {
        kooch_editor_core::keep_all_frames(&mut view.lock().unwrap());
    }

    let sink_view = Arc::clone(&view);
    let sink = puffin::GlobalProfiler::lock().add_sink(Box::new(move |frame| {
        sink_view.lock().unwrap().add_frame(frame)
    }));

    puffin::set_scopes_on(true);
    // What `puffin_http::Server` does for every client that connects:
    // hand it the whole scope collection. It arrives as the delta of the
    // next frame, which is precisely the frame at risk of being evicted.
    //
    // 🔴 Also what makes the two cases comparable. A scope registers
    // globally the first time it runs, so without this the second call
    // to `record` in the same process would emit no delta at all and
    // would "fail" for a reason that has nothing to do with the ring.
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
///
/// The round trip is the test, not the in-memory view: the live view
/// holds a `ScopeCollection` that `write` does not serialise, so a view
/// that resolves names perfectly can still produce an unreadable file.
fn survives_round_trip(view: &puffin::FrameView) -> bool {
    let mut bytes = Vec::new();
    view.write(&mut bytes).expect("write the capture");
    let read = puffin::FrameView::read(&mut bytes.as_slice()).expect("read the capture back");
    read.scope_collection().fetch_by_name(SCOPE).is_some()
}

/// Both cases, in one test, on purpose.
///
/// 🔴 `GlobalProfiler` is global and `new_frame` feeds **every**
/// installed sink. Two `#[test]`s doing this run on two threads of one
/// process, so each one's sink also collects the other's frames and
/// neither measures what it claims to. Split across two tests, the
/// second one passed for a reason that had nothing to do with the ring.
#[test]
fn a_long_capture_keeps_its_names() {
    // The cause, pinned first. If puffin ever raises `max_recent` or
    // starts serialising the collection, this is the assertion that
    // fails and says `keep_all_frames` can go — nothing else would.
    assert!(
        !survives_round_trip(&record(false)),
        "puffin's default view kept the names across {FRAMES} frames. The ring that made \
         `keep_all_frames` necessary is gone or larger; re-check whether the call is still \
         needed before deleting it.",
    );

    assert!(
        survives_round_trip(&record(true)),
        "a {FRAMES}-frame capture lost the name of {SCOPE}. The names ride in the scope_delta \
         of the first frame received, and the view must keep that frame to write it out.",
    );
}
