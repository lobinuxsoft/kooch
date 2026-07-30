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

use ome_core::frame_pacing::FramePace;

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
mod tests {
    use super::*;

    /// A held movement key is not an animation to egui, so this is the
    /// only thing standing between flying the camera and a slideshow.
    #[test]
    fn driving_the_camera_keeps_the_frames_coming() {
        assert_eq!(
            editor_pace(Duration::MAX, false, None, true),
            FramePace::Continuous,
        );
    }

    #[test]
    fn an_idle_editor_sleeps() {
        assert_eq!(
            editor_pace(Duration::MAX, false, None, false),
            FramePace::Wait,
            "nothing animating, nothing connected: the loop should stop",
        );
    }

    #[test]
    fn play_outranks_an_idle_ui() {
        // The viewport texture changes from the project process; egui has
        // no widget to notice that through.
        assert_eq!(
            editor_pace(Duration::MAX, true, Some(ConnectionState::Connected), false),
            FramePace::Continuous
        );
    }

    #[test]
    fn an_animating_widget_keeps_the_loop_running() {
        assert_eq!(
            editor_pace(Duration::ZERO, false, None, false),
            FramePace::Continuous
        );
    }

    #[test]
    fn a_handshake_in_flight_is_not_slowed_down() {
        assert_eq!(
            editor_pace(
                Duration::MAX,
                false,
                Some(ConnectionState::Connecting),
                false
            ),
            FramePace::Continuous
        );
    }

    #[test]
    fn a_dead_session_does_not_keep_the_editor_awake() {
        assert_eq!(
            editor_pace(Duration::MAX, false, Some(ConnectionState::Failed), false),
            FramePace::Wait
        );
    }

    #[test]
    fn a_connected_project_is_polled_while_the_ui_idles() {
        assert_eq!(
            editor_pace(
                Duration::MAX,
                false,
                Some(ConnectionState::Connected),
                false
            ),
            FramePace::After(REMOTE_IDLE_POLL),
        );
    }

    #[test]
    fn a_ui_deadline_shorter_than_the_poll_wins() {
        let blink = Duration::from_millis(16);
        assert_eq!(
            editor_pace(blink, false, Some(ConnectionState::Connected), false),
            FramePace::After(blink),
            "the remote poll is a floor on wake-ups, not a ceiling",
        );
    }

    #[test]
    fn no_viewports_reads_as_nothing_to_repaint() {
        assert_eq!(
            shortest_repaint_delay(&egui::FullOutput::default()),
            Duration::MAX
        );
    }
}
