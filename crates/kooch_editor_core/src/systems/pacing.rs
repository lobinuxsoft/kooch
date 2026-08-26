//! What the editor tells the loop about the frame after this one (#656).
//!
//! egui already knows: `run_ui` returns a `repaint_delay` per viewport,
//! which is `ZERO` while something animates and `Duration::MAX` when the
//! UI has drawn everything it has. Until this module existed that answer
//! was thrown away and the loop redrew regardless, at a pinned core per
//! process for a still image.
//!
//! Two things egui cannot know are folded in here, because they happen
//! outside the UI:
//!
//! - **Play.** The scene is simulating in the project process and the
//!   viewport texture changes without anyone touching a widget.
//! - **A live remote session.** The project's stdout arrives on a socket,
//!   not as a window event, so a fully asleep editor would hold its
//!   Console output until the user happened to move the mouse. A slow
//!   tick is enough for that and still costs nothing measurable.

use std::time::Duration;

use kooch_core::frame_pacing::FramePace;

use crate::remote_session::ConnectionState;

/// How often an idle editor checks a connected project for output.
///
/// Four times a second: fast enough that a log line does not look stuck,
/// slow enough that the cost does not appear in a power meter.
const REMOTE_IDLE_POLL: Duration = Duration::from_millis(250);

/// The pace the editor is asking for, given what egui reported and what
/// is going on outside it.
pub(crate) fn editor_pace(
    repaint_delay: Duration,
    is_playing: bool,
    remote: Option<ConnectionState>,
    driving_camera: bool,
) -> FramePace {
    // A handshake in flight advances one frame at a time, and it is over
    // in the time it takes a project to boot. Slowing that down to save
    // power would only make opening a project feel worse.
    //
    // `driving_camera` is the one egui cannot see. A held key is not an
    // animation as far as egui is concerned — it reports nothing to
    // repaint — so the loop slept and woke on the operating system's key
    // *repeat*, about 25 times a second. The camera then advanced in
    // visible steps, which is what "not fluid" was (#656 taking something
    // back).
    if is_playing || driving_camera || remote == Some(ConnectionState::Connecting) {
        return FramePace::Continuous;
    }

    let ui = FramePace::from_repaint_delay(repaint_delay);
    if remote == Some(ConnectionState::Connected) {
        ui.most_urgent(FramePace::After(REMOTE_IDLE_POLL))
    } else {
        ui
    }
}

/// The shortest repaint delay any viewport asked for.
///
/// A popup or tooltip can open its own viewport; taking the minimum
/// means the one thing that is animating decides, whichever window it
/// lives in. No viewports at all — a frame that drew nothing — reads as
/// "nothing to repaint".
pub(crate) fn shortest_repaint_delay(output: &egui::FullOutput) -> Duration {
    output
        .viewport_output
        .values()
        .map(|viewport| viewport.repaint_delay)
        .min()
        .unwrap_or(Duration::MAX)
}

#[cfg(test)]
mod tests;
