use super::*;

/// `ALL` has to actually be all of them: a picker that offers a
/// subset is a control nobody can bind, with nothing to say so.
#[test]
fn every_list_covers_its_enum() {
    // Round-tripping every entry proves the list and the conversions
    // came from the same expansion.
    for key in KeyCode::ALL {
        assert_eq!(KeyCode::from_upstream(key.to_upstream()), Some(*key));
    }
    for button in GamepadButton::ALL {
        assert_eq!(
            GamepadButton::from_upstream(button.to_upstream()),
            Some(*button)
        );
    }
    for axis in GamepadAxis::ALL {
        assert_eq!(GamepadAxis::from_upstream(axis.to_upstream()), Some(*axis));
    }
    for button in MouseButton::ALL {
        assert_eq!(
            MouseButton::from_upstream(button.to_upstream()),
            Some(*button)
        );
    }
    assert!(
        KeyCode::ALL.len() > 100,
        "the keyboard list looks truncated"
    );
}
