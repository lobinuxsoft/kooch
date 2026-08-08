use super::*;
use crate::mock_backend::MockInputBackend;

fn snapshot_with_keys(keys: &[KeyCode]) -> InputSnapshot {
    InputSnapshot {
        keys: keys.to_vec(),
        ..Default::default()
    }
}

#[test]
fn a_key_that_appears_in_a_snapshot_reads_as_just_pressed() {
    let mut backend = RemoteInputBackend::new();
    backend.apply(&snapshot_with_keys(&[KeyCode::Space]));

    assert!(backend.is_pressed(KeyCode::Space));
    assert!(backend.just_pressed(KeyCode::Space));
}

#[test]
fn a_key_still_held_next_snapshot_is_no_longer_just_pressed() {
    let mut backend = RemoteInputBackend::new();
    backend.apply(&snapshot_with_keys(&[KeyCode::Space]));
    backend.begin_frame();
    backend.apply(&snapshot_with_keys(&[KeyCode::Space]));

    assert!(backend.is_pressed(KeyCode::Space), "the key is still held");
    assert!(
        !backend.just_pressed(KeyCode::Space),
        "the edge outlived its frame"
    );
}

#[test]
fn a_key_missing_from_the_next_snapshot_reads_as_released() {
    let mut backend = RemoteInputBackend::new();
    backend.apply(&snapshot_with_keys(&[KeyCode::Space]));
    backend.begin_frame();
    backend.apply(&snapshot_with_keys(&[]));

    assert!(!backend.is_pressed(KeyCode::Space));
    assert!(backend.just_released(KeyCode::Space));
}

/// The reason snapshots carry state instead of events.
#[test]
fn a_lost_frame_cannot_leave_a_key_stuck_down() {
    let mut backend = RemoteInputBackend::new();
    backend.apply(&snapshot_with_keys(&[KeyCode::KeyW]));
    backend.begin_frame();

    // The release never arrives; the frame after it does.
    backend.apply(&snapshot_with_keys(&[]));

    assert!(
        !backend.is_pressed(KeyCode::KeyW),
        "state snapshots must self-correct; an event stream would not have"
    );
}

/// A host ticking faster than the editor sends must not expire an
/// edge nobody has read yet — the #711 bug, one process over.
#[test]
fn an_edge_survives_host_frames_that_received_nothing() {
    let mut backend = RemoteInputBackend::new();
    backend.apply(&snapshot_with_keys(&[KeyCode::Space]));

    backend.begin_frame();
    backend.begin_frame();
    backend.begin_frame();

    assert!(
        backend.just_pressed(KeyCode::Space),
        "three host frames with no new snapshot ate the keypress"
    );
}

#[test]
fn an_axis_the_snapshot_stops_mentioning_returns_to_centre() {
    let mut backend = RemoteInputBackend::new();
    let pad = GamepadId(0);
    backend.apply(&InputSnapshot {
        gamepads: vec![GamepadSnapshot {
            id: 0,
            axes: vec![(GamepadAxis::LeftStickX, 0.8)],
            ..Default::default()
        }],
        ..Default::default()
    });
    assert_eq!(backend.axis_value(pad, GamepadAxis::LeftStickX), 0.8);

    backend.apply(&InputSnapshot {
        gamepads: vec![GamepadSnapshot {
            id: 0,
            ..Default::default()
        }],
        ..Default::default()
    });

    assert_eq!(
        backend.axis_value(pad, GamepadAxis::LeftStickX),
        0.0,
        "an omitted axis is centred; merging would leave the stick pushed"
    );
}

#[test]
fn release_all_lets_go_of_everything() {
    let mut backend = RemoteInputBackend::new();
    backend.apply(&snapshot_with_keys(&[KeyCode::KeyW, KeyCode::Space]));

    backend.release_all();

    assert!(!backend.is_pressed(KeyCode::KeyW));
    assert!(backend.just_released(KeyCode::Space));
}

#[test]
fn a_snapshot_round_trips_through_json() {
    let snapshot = InputSnapshot {
        keys: vec![KeyCode::KeyW, KeyCode::Space],
        mouse_position: [12.0, 34.0],
        gamepads: vec![GamepadSnapshot {
            id: 1,
            buttons: vec![GamepadButton::South],
            axes: vec![(GamepadAxis::LeftStickY, -0.5)],
        }],
        ..Default::default()
    };

    let json = serde_json::to_string(&snapshot).unwrap();
    assert_eq!(
        serde_json::from_str::<InputSnapshot>(&json).unwrap(),
        snapshot
    );
}

#[test]
fn reading_a_backend_and_applying_it_reproduces_the_state() {
    let mut source = MockInputBackend::new();
    source.press_key(KeyCode::KeyW);
    source.press_mouse(MouseButton::Left);

    let mut backend = RemoteInputBackend::new();
    backend.apply(&InputSnapshot::from_backend(&source));

    assert!(backend.is_pressed(KeyCode::KeyW));
    assert!(backend.is_mouse_pressed(MouseButton::Left));
}

#[test]
fn an_untouched_backend_reads_as_idle() {
    let source = MockInputBackend::new();
    assert!(InputSnapshot::from_backend(&source).is_idle());
}

#[test]
fn a_held_key_is_not_idle() {
    let mut source = MockInputBackend::new();
    source.press_key(KeyCode::KeyW);
    assert!(!InputSnapshot::from_backend(&source).is_idle());
}
