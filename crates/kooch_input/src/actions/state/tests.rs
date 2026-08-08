use super::*;
use crate::actions::action::{Action, ActionMap};
use crate::actions::binding::Binding;
use crate::ids::{GamepadAxis, GamepadButton, KeyCode};
use crate::mock_backend::MockInputBackend;

/// One action's value right now. `ActionState` used to hold a frame
/// of these; the edge it derived lives on `InputAction` now, so what
/// is left to test here is the evaluation itself.
fn value(map: &ActionMap, name: &str, backend: &MockInputBackend) -> ActionValue {
    let id = map.resolve(name).expect("the map declares this action");
    evaluate(map.action(id).expect("resolved id is in range"), backend)
}

fn map() -> ActionMap {
    ActionMap::new("gameplay")
        .add(
            Action::new("move", ControlType::Vector2)
                .bind_all([
                    Binding::composite(Composite::Vector2 {
                        mode: VectorMode::DigitalNormalized,
                    }),
                    Binding::part(PartName::Up, ControlPath::Key(KeyCode::KeyW)),
                    Binding::part(PartName::Down, ControlPath::Key(KeyCode::KeyS)),
                    Binding::part(PartName::Left, ControlPath::Key(KeyCode::KeyA)),
                    Binding::part(PartName::Right, ControlPath::Key(KeyCode::KeyD)),
                ])
                .bind_all([
                    Binding::composite(Composite::Vector2 {
                        mode: VectorMode::Analog,
                    }),
                    Binding::part(PartName::Up, ControlPath::Axis(GamepadAxis::LeftStickY)),
                    Binding::part(PartName::Right, ControlPath::Axis(GamepadAxis::LeftStickX)),
                ]),
        )
        .add(
            Action::new("jump", ControlType::Button)
                .bind(Binding::to(ControlPath::Key(KeyCode::Space)))
                .bind(Binding::to(ControlPath::Button(GamepadButton::South))),
        )
}

/// The thing roll-a-ball wrote by hand, now free: one action, two
/// devices, and gameplay never learns which answered.
#[test]
fn one_action_answers_for_keyboard_and_pad_alike() {
    let map = map();
    let pad = GamepadId(0);
    let mut backend = MockInputBackend::new();
    backend.add_gamepad(pad);

    backend.press_key(KeyCode::Space);
    assert!(value(&map, "jump", &backend).pressed);

    backend.release_key(KeyCode::Space);
    backend.press_gamepad_button(pad, GamepadButton::South);
    assert!(
        value(&map, "jump", &backend).pressed,
        "the pad did not answer for the same action"
    );
}

/// WASD is one `Vector2`, capped, without the game adding anything.
#[test]
fn wasd_reads_as_one_capped_vector() {
    let map = map();
    let mut backend = MockInputBackend::new();

    backend.press_key(KeyCode::KeyW);
    assert_eq!(value(&map, "move", &backend).vector2(), Vec2::new(0.0, 1.0));

    backend.press_key(KeyCode::KeyD);
    let diagonal = value(&map, "move", &backend).vector2();
    assert!(
        (diagonal.length() - 1.0).abs() < 1e-5,
        "diagonal travels {}× too fast",
        diagonal.length()
    );
}

/// The decision this module documents: the strongest binding wins
/// rather than the two summing into a direction nobody asked for.
#[test]
fn the_most_actuated_binding_wins_rather_than_summing() {
    let map = map();
    let pad = GamepadId(0);
    let mut backend = MockInputBackend::new();
    backend.add_gamepad(pad);

    // Keyboard pushes up (magnitude 1); the stick pushes right, but
    // only part way. Summing would give a diagonal of magnitude 1.17
    // that neither input asked for.
    backend.press_key(KeyCode::KeyW);
    backend.set_axis(pad, GamepadAxis::LeftStickX, 0.6);

    assert_eq!(
        value(&map, "move", &backend).vector2(),
        Vec2::new(0.0, 1.0),
        "expected the keyboard alone — a non-zero x means the two were summed"
    );

    // And the other way round: push the stick past what a key can
    // report and it takes over, with no trace of the key.
    backend.release_key(KeyCode::KeyW);
    backend.set_axis(pad, GamepadAxis::LeftStickX, 0.6);
    let stick = value(&map, "move", &backend).vector2();
    assert!(
        (stick.x - 0.6).abs() < 1e-5 && stick.y.abs() < 1e-5,
        "expected the stick alone, got {stick:?}"
    );
}

/// Two bindings of equal strength: the earlier one wins.
///
/// Arbitrary, but it has to be *decided* rather than left to
/// iteration order — a keyboard and a stick both at full push is the
/// common case, not a corner one, and a coin toss there reads as the
/// input flickering.
#[test]
fn a_tie_goes_to_the_binding_listed_first() {
    let map = map();
    let pad = GamepadId(0);
    let mut backend = MockInputBackend::new();
    backend.add_gamepad(pad);

    backend.press_key(KeyCode::KeyW);
    backend.set_axis(pad, GamepadAxis::LeftStickX, 1.0);
    assert_eq!(
        value(&map, "move", &backend).vector2(),
        Vec2::new(0.0, 1.0),
        "the keyboard composite is listed first, so it wins the tie"
    );
}

/// A half-held stick stays half: normalising it would throw away how
/// far it is actually pushed.
#[test]
fn an_analog_composite_keeps_its_magnitude() {
    let map = map();
    let pad = GamepadId(0);
    let mut backend = MockInputBackend::new();
    backend.add_gamepad(pad);

    backend.set_axis(pad, GamepadAxis::LeftStickX, 0.5);
    assert!(
        (value(&map, "move", &backend).vector2().x - 0.5).abs() < 1e-5,
        "the stick was normalised"
    );
}
