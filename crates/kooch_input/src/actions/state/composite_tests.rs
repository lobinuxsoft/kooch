use super::*;
use crate::actions::action::Action;
use crate::actions::binding::Binding;
use crate::ids::KeyCode;
use crate::mock_backend::MockInputBackend;

fn read(action: Action, backend: &MockInputBackend) -> ActionValue {
    evaluate(&action, backend)
}

fn vector3(mode: VectorMode) -> Action {
    Action::new("a", ControlType::Vector3).bind_all([
        Binding::composite(Composite::Vector3 { mode }),
        Binding::part(PartName::Up, ControlPath::Key(KeyCode::KeyE)),
        Binding::part(PartName::Down, ControlPath::Key(KeyCode::KeyQ)),
        Binding::part(PartName::Left, ControlPath::Key(KeyCode::KeyA)),
        Binding::part(PartName::Right, ControlPath::Key(KeyCode::KeyD)),
        Binding::part(PartName::Forward, ControlPath::Key(KeyCode::KeyW)),
        Binding::part(PartName::Backward, ControlPath::Key(KeyCode::KeyS)),
    ])
}

/// All six parts reach their own axis. Ported from Unity's
/// `Vector3Composite.ReadValue`: `(right-left, up-down, forward-back)`.
#[test]
fn a_3d_composite_reads_all_six_directions() {
    for (key, expected) in [
        (KeyCode::KeyD, Vec3::X),
        (KeyCode::KeyA, -Vec3::X),
        (KeyCode::KeyE, Vec3::Y),
        (KeyCode::KeyQ, -Vec3::Y),
        (KeyCode::KeyW, Vec3::Z),
        (KeyCode::KeyS, -Vec3::Z),
    ] {
        let mut backend = MockInputBackend::new();
        backend.press_key(key);
        let got = read(vector3(VectorMode::DigitalNormalized), &backend).vector;
        assert!(
            got.abs_diff_eq(expected, 1e-5),
            "{key:?} should read {expected:?}, got {got:?}"
        );
    }
}

/// 🔴 Three keys at once is a 1.73× speed boost without normalising —
/// worse than the 1.41× a 2D diagonal costs, and the same bug.
#[test]
fn a_3d_diagonal_is_capped_when_the_parts_are_buttons() {
    let mut backend = MockInputBackend::new();
    for key in [KeyCode::KeyD, KeyCode::KeyE, KeyCode::KeyW] {
        backend.press_key(key);
    }

    let normalized = read(vector3(VectorMode::DigitalNormalized), &backend).vector;
    assert!(
        (normalized.length() - 1.0).abs() < 1e-5,
        "three keys held reads {} long, not 1.0",
        normalized.length()
    );

    let digital = read(vector3(VectorMode::Digital), &backend).vector;
    assert!(
        digital.length() > 1.7,
        "Digital must pass the raw sum through, got {}",
        digital.length()
    );
}

/// A 3D action read flat drops z rather than refusing — what
/// `vector()` promises every existing caller.
#[test]
fn a_3d_action_still_reads_as_2d() {
    let mut backend = MockInputBackend::new();
    backend.press_key(KeyCode::KeyW);
    let value = read(vector3(VectorMode::Analog), &backend);
    assert_eq!(
        value.vector2(),
        Vec2::ZERO,
        "forward is not in the xy plane"
    );
    assert_eq!(value.vector.z, 1.0);
}

/// 🔴 An action's processors run **once, on the value that won** —
/// not on each binding, which is how Unity ends up applying a stick's
/// deadzone twice.
///
/// Two bindings, one processor: whichever answers, the scale is
/// applied exactly once.
#[test]
fn an_actions_processors_run_once_on_the_final_value() {
    use crate::actions::processor::Processor;

    let action = |processors: Vec<Processor>| {
        let mut a = Action::new("a", ControlType::Axis)
            .bind(Binding::to(ControlPath::Key(KeyCode::KeyA)))
            .bind(Binding::to(ControlPath::Key(KeyCode::KeyB)));
        a.processors = processors;
        a
    };

    for key in [KeyCode::KeyA, KeyCode::KeyB] {
        let mut backend = MockInputBackend::new();
        backend.press_key(key);
        let scaled = read(action(vec![Processor::Scale { factor: 3.0 }]), &backend);
        assert_eq!(
            scaled.axis(),
            3.0,
            "{key:?} was scaled {} times, not once",
            scaled.axis() / 3.0
        );
    }
}

/// Binding first, action second — the binding shapes the device, the
/// action shapes the meaning. Reversed, the action's clamp would be
/// re-opened by the binding's scale.
#[test]
fn a_bindings_processors_run_before_its_actions() {
    use crate::actions::processor::Processor;

    let mut binding = Binding::to(ControlPath::Key(KeyCode::Space));
    binding.processors = vec![Processor::Scale { factor: 4.0 }];
    let mut action = Action::new("a", ControlType::Axis).bind(binding);
    action.processors = vec![Processor::Clamp {
        min: -2.0,
        max: 2.0,
    }];

    let mut backend = MockInputBackend::new();
    backend.press_key(KeyCode::Space);
    assert_eq!(
        read(action, &backend).axis(),
        2.0,
        "the action's clamp did not see the binding's scale"
    );
}

/// `pressed` is measured after the action's processors: one scaled to
/// zero is not held, however hard the key is pushed.
#[test]
fn the_pressed_flag_reflects_the_actions_processors() {
    use crate::actions::processor::Processor;

    let mut action =
        Action::new("a", ControlType::Button).bind(Binding::to(ControlPath::Key(KeyCode::Space)));
    action.processors = vec![Processor::Scale { factor: 0.0 }];

    let mut backend = MockInputBackend::new();
    backend.press_key(KeyCode::Space);
    assert!(
        !read(action, &backend).pressed,
        "an action scaled to zero still reported as held"
    );
}

fn modified(composite: Composite, parts: &[(PartName, KeyCode)]) -> Action {
    let mut action = Action::new("a", ControlType::Button).bind(Binding::composite(composite));
    for (name, key) in parts {
        action = action.bind(Binding::part(*name, ControlPath::Key(*key)));
    }
    action
}

/// `Ctrl+S`: the gated part is invisible until the modifier is held.
#[test]
fn one_modifier_gates_its_value() {
    let action = || {
        modified(
            Composite::OneModifier,
            &[
                (PartName::Modifier, KeyCode::ControlLeft),
                (PartName::Value, KeyCode::KeyS),
            ],
        )
    };

    let mut backend = MockInputBackend::new();
    backend.press_key(KeyCode::KeyS);
    assert!(
        !read(action(), &backend).pressed,
        "the value fired without its modifier"
    );

    backend.press_key(KeyCode::ControlLeft);
    assert!(
        read(action(), &backend).pressed,
        "the value did not fire with its modifier held"
    );

    backend.release_key(KeyCode::KeyS);
    assert!(
        !read(action(), &backend).pressed,
        "the modifier alone fired the action"
    );
}

/// Two gates, and **both** are required — an `||` here would make
/// `Ctrl+Shift+S` fire on `Ctrl+S`.
#[test]
fn two_modifiers_both_have_to_be_held() {
    let action = || {
        modified(
            Composite::TwoModifiers,
            &[
                (PartName::Modifier, KeyCode::ControlLeft),
                (PartName::Modifier2, KeyCode::ShiftLeft),
                (PartName::Value, KeyCode::KeyS),
            ],
        )
    };

    let mut backend = MockInputBackend::new();
    backend.press_key(KeyCode::KeyS);
    backend.press_key(KeyCode::ControlLeft);
    assert!(
        !read(action(), &backend).pressed,
        "one of two modifiers was enough"
    );

    backend.press_key(KeyCode::ShiftLeft);
    assert!(read(action(), &backend).pressed, "both held did not fire");
}

/// Every composite declares its parts, and they are unique — a
/// duplicate would make `part()` resolve to whichever came first.
#[test]
fn every_composite_declares_the_parts_it_reads() {
    for composite in Composite::ALL {
        let parts = PartName::of(*composite);
        assert!(
            !parts.is_empty(),
            "{composite:?} declares no parts, so nothing can bind it"
        );
        for (index, part) in parts.iter().enumerate() {
            assert!(
                !parts[..index].contains(part),
                "{composite:?} lists {part:?} twice"
            );
        }
    }
}
