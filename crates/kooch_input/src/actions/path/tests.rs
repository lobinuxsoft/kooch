use super::*;

/// The whole point: a binding written today has to mean the same
/// thing next session, on a different pad, in a different order.
#[test]
fn a_binding_names_no_particular_device() {
    let jump = ControlPath::Button(GamepadButton::South);
    let encoded = ron::to_string(&jump).expect("serialise");
    assert!(
        !encoded.contains(char::is_numeric),
        "a device index leaked into the binding: {encoded}"
    );
    assert_eq!(
        ron::from_str::<ControlPath>(&encoded).expect("round trip"),
        jump
    );
}

#[test]
fn every_path_knows_its_device_and_whether_it_is_digital() {
    assert_eq!(
        ControlPath::Key(KeyCode::Space).device(),
        DeviceClass::Keyboard
    );
    assert_eq!(
        ControlPath::Button(GamepadButton::South).device(),
        DeviceClass::Gamepad
    );
    assert_eq!(
        ControlPath::Axis(GamepadAxis::LeftStickX).device(),
        DeviceClass::Gamepad
    );

    assert!(ControlPath::Key(KeyCode::Space).is_digital());
    assert!(!ControlPath::Axis(GamepadAxis::LeftStickX).is_digital());
}
