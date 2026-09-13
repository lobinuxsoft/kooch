//! What the editor tells the loop about the frame after this one (#656).

use std::time::Duration;

use kooch_core::frame_pacing::FramePace;

use crate::remote_session::ConnectionState;

/// How often an idle editor checks a connected project for output.
const REMOTE_IDLE_POLL: Duration = Duration::from_millis(250);

/// The pace the editor is asking for, given what egui reported and what
/// is going on outside it.
pub(crate) fn editor_pace(
    repaint_delay: Duration,
    is_playing: bool,
    remote: Option<ConnectionState>,
    driving_camera: bool,
) -> FramePace {
    // A handshake in flight advances one frame at a time, and it is over in the time it takes a
    // project to boot. Slowing that down to save power would only make opening a project feel
    // worse.
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
