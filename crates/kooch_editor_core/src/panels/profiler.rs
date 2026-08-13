//! The profiler panel (#785) — where the frame actually goes.
//!
//! # Two sources, and only one of them is the point
//!
//! [`local`] reads **this process**: the editor, on this machine, plugged
//! in. It is what shipped first and it answers questions about the
//! editor.
//!
//! [`remote`] reads a **game** over TCP, which is the measurement the
//! whole graphics roadmap is waiting on — 72 FPS at 10 W on the OneXFly
//! is 13.9 ms per frame, and nothing measured on a desktop says anything
//! about it. The game opens the socket (`kooch::profiler`); this end
//! connects to it.
//!
//! # Why this file is thin
//!
//! Because the profiler was **adopted, not written**. The flamegraph, the
//! timeline, the frame history, the scope statistics and the play/pause
//! control all come from `puffin_egui`, and the transport is
//! `puffin_http`. What is here is the source selector, a capture that
//! survives the session, and saying something useful when the feature is
//! off.

use egui::Ui;

#[cfg(feature = "profiling")]
mod local;
#[cfg(feature = "profiling")]
mod remote;

/// Frames left before re-asking puffin for the full scope snapshot.
///
/// 🔴 Asking once, at the moment recording starts, does not work, and
/// the reason is a lost flag inside puffin:
///
/// ```text
/// let propagate_full_delta = std::mem::take(&mut self.propagate_all_scope_details);
/// ...
/// Err(Error::Empty) => return,   // frame had no scopes
/// ```
///
/// The flag is TAKEN before the frame is built, and a frame that comes
/// out empty returns early — carrying the request away with it. The
/// frame that closes right after recording is switched on is exactly
/// that empty frame, because it ran while scopes were off.
///
/// The symptom is a capture where every scope reads `scope#ScopeId(67)`
/// except the handful registered after the fact. It does not look
/// broken; it is just unusable.
///
/// So the request is repeated for the first couple of seconds of
/// recording. Two frames was not enough: a scope only registers the
/// first time it runs, so anything that happens once every few frames —
/// or the first time you open a panel — registers after the snapshot has
/// already gone out, and comes back nameless. Repeating it costs one
/// atomic and one bool per frame for two seconds, against a capture that
/// is silently unreadable.
///
/// 🟢 None of this applies to [`remote`]: `puffin_http::Server` keeps its
/// own `ScopeCollection` and re-sends all of it to every client that
/// connects, so a viewer attached an hour in still gets names.
#[cfg(feature = "profiling")]
pub(crate) static SNAPSHOT_COUNTDOWN: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);

/// Which process the panel is showing.
///
/// A plain atomic rather than editor state: the panel is drawn from a
/// free function with no state of its own, and the two sources each own
/// theirs already.
#[cfg(feature = "profiling")]
static SHOW_REMOTE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Draws the profiler, or the reason there is none.
pub fn draw_profiler_content(ui: &mut Ui) {
    #[cfg(feature = "profiling")]
    {
        use std::sync::atomic::Ordering;

        let mut remote_selected = SHOW_REMOTE.load(Ordering::Relaxed);
        ui.horizontal(|ui| {
            ui.label("Profiling:");
            ui.selectable_value(&mut remote_selected, false, "This editor")
                .on_hover_text("The process drawing this window, on this machine");
            ui.selectable_value(&mut remote_selected, true, "A running game")
                .on_hover_text(
                    "A game built with --features profiling, over the network. \
                     The only source that can answer the 10 W question.",
                );
        });
        SHOW_REMOTE.store(remote_selected, Ordering::Relaxed);
        ui.separator();

        if remote_selected {
            remote::draw(ui);
        } else {
            local::draw(ui);
        }
    }

    #[cfg(not(feature = "profiling"))]
    {
        let _ = ui;
        ui.heading("This editor was built without its profiler");
        ui.add_space(8.0);
        ui.label(
            "That is not the normal state: the feature is on by default, because an editor \
             that cannot answer \"why is this frame slow\" is missing the tool this one is \
             built around. Something passed --no-default-features.",
        );
        ui.add_space(8.0);
        ui.code("cargo run -p kooch_editor");
        ui.add_space(8.0);
        ui.label(
            "A shipped game is the opposite case and stays that way: its instrumentation is \
             opt-in, so a release carries none of it — not switched off, absent (#558).",
        );
        ui.add_space(8.0);
        ui.label(
            "To profile a game on the target hardware, build it with the profiling preset \
             from the Build panel and connect to it from the desktop.",
        );
    }
}
