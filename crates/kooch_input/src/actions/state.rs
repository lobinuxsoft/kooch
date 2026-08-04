//! Reading a [`ActionMap`] against a backend, once per frame.
//!
//! # Why the most actuated binding wins
//!
//! An action with several bindings has to decide what happens when more
//! than one is actuated. Two answers are defensible:
//!
//! - **Sum them.** What roll-a-ball did by hand: add the keyboard's
//!   direction to the stick's and cap the result. Holding `W` while
//!   pushing the stick right produces a diagonal nobody asked for.
//! - **The strongest wins.** What Unity does. `W` and a stick pushed
//!   further apart give the stick; `W` alone gives `W`.
//!
//! The second is taken, for a reason beyond taste: the same "who wins"
//! machinery is what lets a map on top **consume** an action so the map
//! below stops seeing it (see [`ActionMap::priority`]). Two mechanisms
//! for one question would drift.
//!
//! # State, never events
//!
//! [`ActionState`] holds what is true *now*, and edges are derived by
//! comparing against the previous frame. A dropped frame therefore
//! self-corrects, where a queue of events would leave an action stuck
//! down forever — the failure that #711 and #713 both were, once at the
//! backend and once across the wire.

use glam::{Vec2, Vec3};

use super::action::{Action, ActionId, ActionMap, ControlType};
use super::binding::{Binding, BothHeld, Composite, Group, PartName, VectorMode, groups};
use super::path::ControlPath;
use crate::backend::InputBackend;
use crate::ids::GamepadId;

/// What one action is worth this frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ActionValue {
    /// The full value. A button uses `x` as 0 or 1, an axis uses `x`,
    /// a 2D composite `xy`. Three components so a 3D composite has
    /// somewhere to land — see [`ControlType::Vector3`].
    pub vector: Vec3,
    /// Whether it counts as held. For an axis, past halfway.
    pub pressed: bool,
}

impl ActionValue {
    pub fn axis(self) -> f32 {
        self.vector.x
    }

    /// The first two components. What a 2D action wants, and what a 3D
    /// one gives up when read as flat.
    pub fn vector2(self) -> Vec2 {
        self.vector.truncate()
    }
}

/// Reads one action against a backend, with no map involved.
///
/// A map is a way to group actions that turn on and off together; it is
/// not a requirement for evaluating one. Unity draws the same line — an
/// action can "stand on its own", and internally it wraps it in a map of
/// one, because *"to the action system, there are no actions without
/// action maps"*. Here there is no wrapper: the evaluator never needed
/// the map, only the action.
pub fn evaluate(action: &Action, backend: &dyn InputBackend) -> ActionValue {
    let pad = backend.gamepads().first().copied();
    read_action(action, backend, pad)
}

/// Reads every binding group and keeps the most actuated.
fn read_action(action: &Action, backend: &dyn InputBackend, pad: Option<GamepadId>) -> ActionValue {
    let control_type = action.control_type;
    let mut best = Vec3::ZERO;
    let mut best_magnitude = 0.0;

    for group in groups(&action.bindings) {
        let raw = match group {
            Group::Single { path, binding } => {
                let value = read_control(path, backend, pad);
                apply(binding, Vec3::new(value, 0.0, 0.0))
            }
            Group::Composite {
                composite,
                head,
                parts,
            } => apply(head, read_composite(composite, parts, backend, pad)),
        };
        let magnitude = raw.length();
        if magnitude > best_magnitude {
            best_magnitude = magnitude;
            best = raw;
        }
    }

    // The action's own processors, run once on the value that won rather
    // than on each binding. Unity applies its equivalents per binding,
    // which is how a stick ends up with two deadzones; once at the end
    // there is nothing to double, and a normalize or a sensitivity is
    // written in one place instead of on every binding.
    let value = action
        .processors
        .iter()
        .fold(best, |acc, processor| processor.apply_vec3(acc));

    ActionValue {
        vector: value,
        // Measured after those processors, not before: an action scaled
        // to zero is not held, and one clamped up is.
        pressed: match control_type {
            ControlType::Vector2 | ControlType::Vector3 => value.length() > 0.5,
            _ => value.x.abs() > 0.5,
        },
    }
}

/// Runs a binding's processors, in order.
fn apply(binding: &Binding, value: Vec3) -> Vec3 {
    binding
        .processors
        .iter()
        .fold(value, |acc, processor| processor.apply_vec3(acc))
}

fn read_composite(
    composite: Composite,
    parts: &[Binding],
    backend: &dyn InputBackend,
    pad: Option<GamepadId>,
) -> Vec3 {
    let part = |name: PartName| -> f32 {
        parts
            .iter()
            .find(|binding| matches!(binding.role, super::binding::Role::Part { name: n, .. } if n == name))
            .and_then(|binding| binding.path().map(|path| (binding, path)))
            .map(|(binding, path)| {
                apply(binding, Vec3::new(read_control(path, backend, pad), 0.0, 0.0)).x
            })
            .unwrap_or(0.0)
    };

    match composite {
        Composite::Axis1D { both_held } => {
            let positive = part(PartName::Positive);
            let negative = part(PartName::Negative);
            let both = positive.abs() > 0.5 && negative.abs() > 0.5;
            let value = match (both, both_held) {
                (true, BothHeld::Neither) => 0.0,
                (true, BothHeld::Positive) => positive,
                (true, BothHeld::Negative) => -negative,
                (false, _) => positive - negative,
            };
            Vec3::new(value, 0.0, 0.0)
        }
        Composite::Vector2 { mode } => {
            let (up, down) = (part(PartName::Up), part(PartName::Down));
            let (left, right) = (part(PartName::Left), part(PartName::Right));
            normalized(Vec3::new(right - left, up - down, 0.0), mode)
        }
        Composite::Vector3 { mode } => {
            let (up, down) = (part(PartName::Up), part(PartName::Down));
            let (left, right) = (part(PartName::Left), part(PartName::Right));
            let (forward, back) = (part(PartName::Forward), part(PartName::Backward));
            normalized(Vec3::new(right - left, up - down, forward - back), mode)
        }
        // The gate reads as a button even when bound to an axis, matching
        // Unity: a trigger half-pulled is not a held modifier.
        Composite::OneModifier => gated(part(PartName::Modifier) > 0.5, part(PartName::Value)),
        Composite::TwoModifiers => gated(
            part(PartName::Modifier) > 0.5 && part(PartName::Modifier2) > 0.5,
            part(PartName::Value),
        ),
    }
}

/// Caps a composite's raw sum at length 1 when its parts are buttons.
///
/// Without it a diagonal travels 1.41× faster than a straight line —
/// 1.73× in three dimensions, where three keys can be held at once.
fn normalized(raw: Vec3, mode: VectorMode) -> Vec3 {
    match mode {
        // A stick already reports how far it is pushed; normalising
        // would throw that away.
        VectorMode::Analog | VectorMode::Digital => raw,
        VectorMode::DigitalNormalized => {
            if raw.length_squared() > 1.0 {
                raw.normalize()
            } else {
                raw
            }
        }
    }
}

/// A modifier composite's value: the gated part, or nothing.
fn gated(open: bool, value: f32) -> Vec3 {
    if open {
        Vec3::new(value, 0.0, 0.0)
    } else {
        Vec3::ZERO
    }
}

/// One control's current value, as a number.
///
/// A button reads 0 or 1, so a button bound where an axis is expected
/// behaves like a stick pushed fully — which is what makes a d-pad and a
/// stick interchangeable in a binding.
fn read_control(path: ControlPath, backend: &dyn InputBackend, pad: Option<GamepadId>) -> f32 {
    match path {
        ControlPath::Key(key) => backend.is_pressed(key) as u8 as f32,
        ControlPath::Mouse(button) => backend.is_mouse_pressed(button) as u8 as f32,
        ControlPath::Button(button) => pad
            .map(|pad| backend.is_button_pressed(pad, button) as u8 as f32)
            .unwrap_or(0.0),
        ControlPath::Axis(axis) => pad.map(|pad| backend.axis_value(pad, axis)).unwrap_or(0.0),
    }
}

#[cfg(test)]
mod tests {
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
}

#[cfg(test)]
mod composite_tests {
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

        let mut action = Action::new("a", ControlType::Button)
            .bind(Binding::to(ControlPath::Key(KeyCode::Space)));
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
}
