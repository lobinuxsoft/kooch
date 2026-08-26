use super::*;

#[test]
fn pressed_state_round_trips() {
    let mut backend = MockInputBackend::new();
    assert!(!backend.is_pressed(KeyCode::Space));
    backend.press_key(KeyCode::Space);
    assert!(backend.is_pressed(KeyCode::Space));
    backend.release_key(KeyCode::Space);
    assert!(!backend.is_pressed(KeyCode::Space));
}

#[test]
fn just_pressed_clears_on_the_next_frame() {
    let mut backend = MockInputBackend::new();
    backend.press_key(KeyCode::KeyA);
    assert!(backend.just_pressed(KeyCode::KeyA));
    backend.begin_frame();
    assert!(!backend.just_pressed(KeyCode::KeyA));
    // Still pressed though.
    assert!(backend.is_pressed(KeyCode::KeyA));
}

#[test]
fn polling_does_not_expire_this_frames_edges() {
    let mut backend = MockInputBackend::new();
    backend.press_key(KeyCode::KeyA);

    backend.poll();

    assert!(
        backend.just_pressed(KeyCode::KeyA),
        "poll ate the edge, which is the whole reason no game could read a keypress"
    );
}

#[test]
fn just_released_clears_on_the_next_frame() {
    let mut backend = MockInputBackend::new();
    backend.press_key(KeyCode::KeyB);
    backend.begin_frame();
    backend.release_key(KeyCode::KeyB);
    assert!(backend.just_released(KeyCode::KeyB));
    backend.begin_frame();
    assert!(!backend.just_released(KeyCode::KeyB));
}

#[test]
fn poll_returns_queued_events_then_clears() {
    let mut backend = MockInputBackend::new();
    backend.press_key(KeyCode::KeyA);
    backend.press_key(KeyCode::KeyB);
    let events = backend.poll();
    assert_eq!(events.len(), 2);
    let next = backend.poll();
    assert!(next.is_empty());
}

#[test]
fn mouse_delta_accumulates_then_resets() {
    let mut backend = MockInputBackend::new();
    backend.move_mouse_to(Vec2::new(10.0, 20.0));
    backend.move_mouse_to(Vec2::new(15.0, 30.0));
    assert_eq!(backend.mouse_delta(), Vec2::new(15.0, 30.0));
    backend.begin_frame();
    assert_eq!(backend.mouse_delta(), Vec2::ZERO);
    // Position stays.
    assert_eq!(backend.mouse_position(), Vec2::new(15.0, 30.0));
}

#[test]
fn pressed_keys_snapshot_lists_held_only() {
    let mut backend = MockInputBackend::new();
    backend.press_key(KeyCode::KeyA);
    backend.press_key(KeyCode::KeyB);
    backend.release_key(KeyCode::KeyA);
    let snapshot = backend.pressed_keys();
    assert!(!snapshot.contains(&KeyCode::KeyA));
    assert!(snapshot.contains(&KeyCode::KeyB));
    assert_eq!(snapshot.len(), 1);
}

#[test]
fn gamepad_axes_clamp_to_range() {
    let mut backend = MockInputBackend::new();
    let id = unsafe { std::mem::zeroed::<GamepadId>() };
    backend.add_gamepad(id);
    backend.set_axis(id, GamepadAxis::LeftStickX, 2.0);
    assert_eq!(backend.axis_value(id, GamepadAxis::LeftStickX), 1.0);
    backend.set_axis(id, GamepadAxis::LeftStickX, -3.0);
    assert_eq!(backend.axis_value(id, GamepadAxis::LeftStickX), -1.0);
}

/// A held button must read as pressed once, not every frame.
///
/// Without this the keyboard and the gamepad disagree: `just_pressed`
/// existed for keys and not for buttons, so gameplay wanting "on
/// press" had to settle for "while held" on a pad. Written into a
/// per-frame jump intent that is an impulse every frame the button is
/// down — the jump that feels right on a keyboard fires the player
/// off the map on a controller (#57).
#[test]
fn a_held_button_reads_as_just_pressed_only_once() {
    let pad = GamepadId(0);
    let mut backend = MockInputBackend::new();
    backend.add_gamepad(pad);

    backend.begin_frame();
    backend.press_gamepad_button(pad, GamepadButton::South);
    assert!(backend.just_button_pressed(pad, GamepadButton::South));
    assert!(backend.is_button_pressed(pad, GamepadButton::South));

    backend.begin_frame();
    assert!(
        !backend.just_button_pressed(pad, GamepadButton::South),
        "a held button fired twice — this is the jump that launches the player"
    );
    assert!(
        backend.is_button_pressed(pad, GamepadButton::South),
        "the button stopped reading as held"
    );

    backend.begin_frame();
    backend.release_gamepad_button(pad, GamepadButton::South);
    assert!(backend.just_button_released(pad, GamepadButton::South));
    assert!(!backend.is_button_pressed(pad, GamepadButton::South));

    backend.begin_frame();
    assert!(!backend.just_button_released(pad, GamepadButton::South));
}

/// The keyboard and the gamepad have to answer the same question the
/// same way, or gameplay has to special-case which device it is on —
/// which is exactly what roll-a-ball's `jump_requested` had to do.
#[test]
fn a_button_behaves_like_a_key() {
    let pad = GamepadId(0);
    let mut backend = MockInputBackend::new();
    backend.add_gamepad(pad);

    backend.begin_frame();
    backend.press_key(KeyCode::Space);
    backend.press_gamepad_button(pad, GamepadButton::South);
    assert_eq!(
        backend.just_pressed(KeyCode::Space),
        backend.just_button_pressed(pad, GamepadButton::South),
    );

    backend.begin_frame();
    assert_eq!(
        backend.just_pressed(KeyCode::Space),
        backend.just_button_pressed(pad, GamepadButton::South),
        "held: the key expired its edge and the button did not"
    );
}
