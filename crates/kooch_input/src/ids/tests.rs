use super::*;

#[test]
fn a_key_survives_the_round_trip_through_winit() {
    for key in [
        KeyCode::KeyW,
        KeyCode::Space,
        KeyCode::ArrowUp,
        KeyCode::F12,
        KeyCode::NumpadEnter,
    ] {
        assert_eq!(KeyCode::from_upstream(key.to_upstream()), Some(key));
    }
}

#[test]
fn a_button_survives_the_round_trip_through_gilrs() {
    for button in [
        GamepadButton::South,
        GamepadButton::DPadUp,
        GamepadButton::Start,
    ] {
        assert_eq!(
            GamepadButton::from_upstream(button.to_upstream()),
            Some(button)
        );
    }
}

#[test]
fn an_axis_survives_the_round_trip_through_gilrs() {
    for axis in [
        GamepadAxis::LeftStickX,
        GamepadAxis::RightStickY,
        GamepadAxis::DPadX,
    ] {
        assert_eq!(GamepadAxis::from_upstream(axis.to_upstream()), Some(axis));
    }
}

/// gilrs has an `Unknown` variant and this does not, on purpose.
#[test]
fn an_input_we_have_no_name_for_is_none_rather_than_a_catch_all() {
    assert_eq!(GamepadButton::from_upstream(gilrs::Button::Unknown), None);
    assert_eq!(GamepadAxis::from_upstream(gilrs::Axis::Unknown), None);
}

/// The point of the whole module: this is what gilrs cannot do.
#[test]
fn a_gamepad_id_can_be_built_from_a_number() {
    assert_eq!(GamepadId(3).index(), 3);
}

#[test]
fn identifiers_serialise_by_name_so_a_file_reads_as_text() {
    let json = serde_json::to_string(&KeyCode::KeyW).unwrap();
    assert_eq!(json, "\"KeyW\"");
    assert_eq!(
        serde_json::from_str::<KeyCode>(&json).unwrap(),
        KeyCode::KeyW
    );
}
