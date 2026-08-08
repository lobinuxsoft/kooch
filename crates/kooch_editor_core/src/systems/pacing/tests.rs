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
