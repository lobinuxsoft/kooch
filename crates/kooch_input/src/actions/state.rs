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

use glam::Vec2;

use super::action::{ActionId, ActionMap, ControlType};
use super::binding::{Binding, BothHeld, Composite, Group, PartName, Vector2Mode, groups};
use super::path::ControlPath;
use crate::backend::InputBackend;
use crate::ids::GamepadId;

/// What one action is worth this frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ActionValue {
    /// The full value. A button uses `x` as 0 or 1; an axis uses `x`.
    pub vector: Vec2,
    /// Whether it counts as held. For an axis, past halfway.
    pub pressed: bool,
}

impl ActionValue {
    pub fn axis(self) -> f32 {
        self.vector.x
    }
}

/// Every action's value this frame, and the previous frame's, so edges
/// come from comparing rather than from remembering events.
#[derive(Debug, Default, Clone)]
pub struct ActionState {
    current: Vec<ActionValue>,
    previous: Vec<ActionValue>,
}

impl ActionState {
    /// Sized for a map. Resizing on the fly would make the id of an
    /// action depend on when it was first read.
    pub fn for_map(map: &ActionMap) -> Self {
        Self {
            current: vec![ActionValue::default(); map.actions.len()],
            previous: vec![ActionValue::default(); map.actions.len()],
        }
    }

    /// Reads every action in `map` from `backend`.
    ///
    /// The whole frame's worth in one pass over contiguous arrays: no
    /// lookup by name, no allocation, and the order is the map's order.
    pub fn update(&mut self, map: &ActionMap, backend: &dyn InputBackend) {
        if self.current.len() != map.actions.len() {
            *self = Self::for_map(map);
        }
        std::mem::swap(&mut self.previous, &mut self.current);

        let pad = backend.gamepads().first().copied();
        for (index, action) in map.actions.iter().enumerate() {
            self.current[index] = read_action(action.control_type, &action.bindings, backend, pad);
        }
    }

    pub fn value(&self, id: ActionId) -> ActionValue {
        self.current.get(id.index()).copied().unwrap_or_default()
    }

    pub fn axis(&self, id: ActionId) -> f32 {
        self.value(id).axis()
    }

    pub fn vector(&self, id: ActionId) -> Vec2 {
        self.value(id).vector
    }

    pub fn pressed(&self, id: ActionId) -> bool {
        self.value(id).pressed
    }

    /// True only on the frame it went down.
    ///
    /// Derived, not recorded — which is what makes it correct for a
    /// keyboard and a gamepad alike, and across a remote link where the
    /// two ends do not tick together.
    pub fn just_pressed(&self, id: ActionId) -> bool {
        let was = self.previous.get(id.index()).copied().unwrap_or_default();
        self.pressed(id) && !was.pressed
    }

    pub fn just_released(&self, id: ActionId) -> bool {
        let was = self.previous.get(id.index()).copied().unwrap_or_default();
        !self.pressed(id) && was.pressed
    }
}

/// Reads every binding group and keeps the most actuated.
fn read_action(
    control_type: ControlType,
    bindings: &[Binding],
    backend: &dyn InputBackend,
    pad: Option<GamepadId>,
) -> ActionValue {
    let mut best = Vec2::ZERO;
    let mut best_magnitude = 0.0;

    for group in groups(bindings) {
        let raw = match group {
            Group::Single { path, binding } => {
                let value = read_control(path, backend, pad);
                apply(binding, Vec2::new(value, 0.0))
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

    ActionValue {
        vector: best,
        // Past halfway for anything continuous; a button is already 0 or
        // 1 so the same threshold reads it correctly.
        pressed: match control_type {
            ControlType::Vector2 => best_magnitude > 0.5,
            _ => best.x.abs() > 0.5,
        },
    }
}

/// Runs a binding's processors, in order.
fn apply(binding: &Binding, value: Vec2) -> Vec2 {
    binding
        .processors
        .iter()
        .fold(value, |acc, processor| processor.apply_vec2(acc))
}

fn read_composite(
    composite: Composite,
    parts: &[Binding],
    backend: &dyn InputBackend,
    pad: Option<GamepadId>,
) -> Vec2 {
    let part = |name: PartName| -> f32 {
        parts
            .iter()
            .find(|binding| matches!(binding.role, super::binding::Role::Part { name: n, .. } if n == name))
            .and_then(|binding| binding.path().map(|path| (binding, path)))
            .map(|(binding, path)| apply(binding, Vec2::new(read_control(path, backend, pad), 0.0)).x)
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
            Vec2::new(value, 0.0)
        }
        Composite::Vector2 { mode } => {
            let (up, down) = (part(PartName::Up), part(PartName::Down));
            let (left, right) = (part(PartName::Left), part(PartName::Right));
            let raw = Vec2::new(right - left, up - down);
            match mode {
                // A stick already reports how far it is pushed.
                Vector2Mode::Analog | Vector2Mode::Digital => raw,
                // Buttons do not, so a diagonal would be 1.41× too fast.
                Vector2Mode::DigitalNormalized => {
                    if raw.length_squared() > 1.0 {
                        raw.normalize()
                    } else {
                        raw
                    }
                }
            }
        }
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

    fn map() -> ActionMap {
        ActionMap::new("gameplay")
            .add(
                Action::new("move", ControlType::Vector2)
                    .bind_all([
                        Binding::composite(Composite::Vector2 {
                            mode: Vector2Mode::DigitalNormalized,
                        }),
                        Binding::part(PartName::Up, ControlPath::Key(KeyCode::KeyW)),
                        Binding::part(PartName::Down, ControlPath::Key(KeyCode::KeyS)),
                        Binding::part(PartName::Left, ControlPath::Key(KeyCode::KeyA)),
                        Binding::part(PartName::Right, ControlPath::Key(KeyCode::KeyD)),
                    ])
                    .bind_all([
                        Binding::composite(Composite::Vector2 {
                            mode: Vector2Mode::Analog,
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
        let jump = map.resolve("jump").unwrap();
        let pad = GamepadId(0);
        let mut backend = MockInputBackend::new();
        backend.add_gamepad(pad);
        let mut state = ActionState::for_map(&map);

        backend.press_key(KeyCode::Space);
        state.update(&map, &backend);
        assert!(state.pressed(jump) && state.just_pressed(jump));

        backend.release_key(KeyCode::Space);
        backend.press_gamepad_button(pad, GamepadButton::South);
        state.update(&map, &backend);
        assert!(
            state.pressed(jump),
            "the pad did not answer for the same action"
        );
    }

    /// Edges come from comparing frames, so a held input fires once — the
    /// bug that launched the ball off the map, now impossible to write.
    #[test]
    fn a_held_action_is_pressed_once() {
        let map = map();
        let jump = map.resolve("jump").unwrap();
        let mut backend = MockInputBackend::new();
        let mut state = ActionState::for_map(&map);

        backend.press_key(KeyCode::Space);
        state.update(&map, &backend);
        assert!(state.just_pressed(jump));

        state.update(&map, &backend);
        assert!(state.pressed(jump), "still held");
        assert!(!state.just_pressed(jump), "a held action fired twice");

        backend.release_key(KeyCode::Space);
        state.update(&map, &backend);
        assert!(state.just_released(jump));
    }

    /// WASD is one `Vector2`, capped, without the game adding anything.
    #[test]
    fn wasd_reads_as_one_capped_vector() {
        let map = map();
        let mv = map.resolve("move").unwrap();
        let mut backend = MockInputBackend::new();
        let mut state = ActionState::for_map(&map);

        backend.press_key(KeyCode::KeyW);
        state.update(&map, &backend);
        assert_eq!(state.vector(mv), Vec2::new(0.0, 1.0));

        backend.press_key(KeyCode::KeyD);
        state.update(&map, &backend);
        let diagonal = state.vector(mv);
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
        let mv = map.resolve("move").unwrap();
        let pad = GamepadId(0);
        let mut backend = MockInputBackend::new();
        backend.add_gamepad(pad);
        let mut state = ActionState::for_map(&map);

        // Keyboard pushes up (magnitude 1); the stick pushes right, but
        // only part way. Summing would give a diagonal of magnitude 1.17
        // that neither input asked for.
        backend.press_key(KeyCode::KeyW);
        backend.set_axis(pad, GamepadAxis::LeftStickX, 0.6);
        state.update(&map, &backend);

        let value = state.vector(mv);
        assert_eq!(
            value,
            Vec2::new(0.0, 1.0),
            "expected the keyboard alone — a non-zero x means the two were summed"
        );

        // And the other way round: push the stick past what a key can
        // report and it takes over, with no trace of the key.
        backend.release_key(KeyCode::KeyW);
        backend.set_axis(pad, GamepadAxis::LeftStickX, 0.6);
        state.update(&map, &backend);
        let value = state.vector(mv);
        assert!(
            (value.x - 0.6).abs() < 1e-5 && value.y.abs() < 1e-5,
            "expected the stick alone, got {value:?}"
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
        let mv = map.resolve("move").unwrap();
        let pad = GamepadId(0);
        let mut backend = MockInputBackend::new();
        backend.add_gamepad(pad);
        let mut state = ActionState::for_map(&map);

        backend.press_key(KeyCode::KeyW);
        backend.set_axis(pad, GamepadAxis::LeftStickX, 1.0);
        state.update(&map, &backend);
        assert_eq!(
            state.vector(mv),
            Vec2::new(0.0, 1.0),
            "the keyboard composite is listed first, so it wins the tie"
        );
    }

    /// A half-held stick stays half: normalising it would throw away how
    /// far it is actually pushed.
    #[test]
    fn an_analog_composite_keeps_its_magnitude() {
        let map = map();
        let mv = map.resolve("move").unwrap();
        let pad = GamepadId(0);
        let mut backend = MockInputBackend::new();
        backend.add_gamepad(pad);
        let mut state = ActionState::for_map(&map);

        backend.set_axis(pad, GamepadAxis::LeftStickX, 0.5);
        state.update(&map, &backend);
        assert!(
            (state.vector(mv).x - 0.5).abs() < 1e-5,
            "the stick was normalised"
        );
    }
}
