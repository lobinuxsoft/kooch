use super::*;
use crate::ids::{GamepadAxis, GamepadButton, KeyCode};

fn wasd() -> Vec<Binding> {
    vec![
        Binding::composite(Composite::Vector2 {
            mode: VectorMode::DigitalNormalized,
        }),
        Binding::part(PartName::Up, ControlPath::Key(KeyCode::KeyW)),
        Binding::part(PartName::Down, ControlPath::Key(KeyCode::KeyS)),
        Binding::part(PartName::Left, ControlPath::Key(KeyCode::KeyA)),
        Binding::part(PartName::Right, ControlPath::Key(KeyCode::KeyD)),
    ]
}

/// The flat list has to read back as the structure it encodes.
#[test]
fn a_composite_gathers_the_parts_that_follow_it() {
    let mut bindings = wasd();
    bindings.push(Binding::to(ControlPath::Button(GamepadButton::South)));

    let groups = groups(&bindings);
    assert_eq!(groups.len(), 2, "expected one composite and one single");

    match &groups[0] {
        Group::Composite { parts, .. } => assert_eq!(parts.len(), 4),
        other => panic!("expected a composite, got {other:?}"),
    }
    assert!(matches!(groups[1], Group::Single { .. }));
}

/// Two composites on one action — keyboard and stick both driving
/// "move" — must not bleed into each other.
#[test]
fn a_second_composite_starts_a_new_group() {
    let mut bindings = wasd();
    bindings.push(Binding::composite(Composite::Vector2 {
        mode: VectorMode::Analog,
    }));
    bindings.push(Binding::part(
        PartName::Up,
        ControlPath::Axis(GamepadAxis::LeftStickY),
    ));

    let groups = groups(&bindings);
    assert_eq!(groups.len(), 2);
    match (&groups[0], &groups[1]) {
        (Group::Composite { parts: a, .. }, Group::Composite { parts: b, .. }) => {
            assert_eq!(a.len(), 4, "the keyboard composite swallowed the stick's");
            assert_eq!(b.len(), 1);
        }
        other => panic!("expected two composites, got {other:?}"),
    }
}

/// A malformed list — parts with no head — must not be attributed to
/// a composite nobody wrote.
#[test]
fn orphan_parts_are_dropped_rather_than_guessed_at() {
    let bindings = vec![
        Binding::part(PartName::Up, ControlPath::Key(KeyCode::KeyW)),
        Binding::to(ControlPath::Key(KeyCode::Space)),
    ];
    let groups = groups(&bindings);
    assert_eq!(groups.len(), 1);
    assert!(matches!(groups[0], Group::Single { .. }));
}

/// The default of a 2D composite has to be the one WASD needs, since
/// that is what the editor will hand an author who changes nothing.
#[test]
fn the_defaults_are_the_ones_a_keyboard_wants() {
    assert_eq!(VectorMode::default(), VectorMode::DigitalNormalized);
    assert_eq!(
        BothHeld::default(),
        BothHeld::Neither,
        "left and right together should cancel, not pick a winner"
    );
}

/// The whole model has to survive a round trip, because it is going
/// to live in a file.
#[test]
fn a_binding_list_round_trips() {
    let bindings = wasd();
    let encoded = ron::to_string(&bindings).expect("serialise");
    let decoded: Vec<Binding> = ron::from_str(&encoded).expect("deserialise");
    assert_eq!(decoded, bindings);
}
