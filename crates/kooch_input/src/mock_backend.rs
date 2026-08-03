//! [`MockInputBackend`] — deterministic, headless input source for tests.
//!
//! Game code that depends on `Box<dyn InputBackend>` can be exercised
//! without winit / gilrs by inserting a `MockInputBackend` and driving
//! its setters directly.

use glam::Vec2;
use std::collections::{HashMap, HashSet};

use crate::backend::{
    GamepadAxis, GamepadButton, GamepadId, InputBackend, InputEvent, KeyCode, MouseButton,
};

/// Test-friendly backend with direct setters for each piece of state.
///
/// `poll` drains queued events; `begin_frame` is what expires
/// `just_pressed` / `just_released`, exactly as in the real backend.
#[derive(Default)]
pub struct MockInputBackend {
    pressed_keys: HashSet<KeyCode>,
    just_pressed_keys: HashSet<KeyCode>,
    just_released_keys: HashSet<KeyCode>,
    pressed_mouse: HashSet<MouseButton>,
    mouse_position: Vec2,
    mouse_delta: Vec2,
    gamepads: HashMap<GamepadId, GamepadState>,
    queued_events: Vec<InputEvent>,
}

#[derive(Default)]
struct GamepadState {
    just_pressed_buttons: HashSet<GamepadButton>,
    just_released_buttons: HashSet<GamepadButton>,
    pressed_buttons: HashSet<GamepadButton>,
    axes: HashMap<GamepadAxis, f32>,
}

impl MockInputBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn press_key(&mut self, key: KeyCode) {
        if self.pressed_keys.insert(key) {
            self.just_pressed_keys.insert(key);
            self.queued_events.push(InputEvent::KeyPressed(key));
        }
    }

    pub fn release_key(&mut self, key: KeyCode) {
        if self.pressed_keys.remove(&key) {
            self.just_released_keys.insert(key);
            self.queued_events.push(InputEvent::KeyReleased(key));
        }
    }

    pub fn press_mouse(&mut self, button: MouseButton) {
        if self.pressed_mouse.insert(button) {
            self.queued_events.push(InputEvent::MousePressed(button));
        }
    }

    pub fn release_mouse(&mut self, button: MouseButton) {
        if self.pressed_mouse.remove(&button) {
            self.queued_events.push(InputEvent::MouseReleased(button));
        }
    }

    pub fn move_mouse_to(&mut self, position: Vec2) {
        let delta = position - self.mouse_position;
        self.mouse_position = position;
        self.mouse_delta += delta;
        self.queued_events
            .push(InputEvent::MouseMoved { position, delta });
    }

    pub fn add_gamepad(&mut self, id: GamepadId) {
        self.gamepads.entry(id).or_default();
        self.queued_events.push(InputEvent::GamepadConnected(id));
    }

    pub fn press_gamepad_button(&mut self, gamepad: GamepadId, button: GamepadButton) {
        let entry = self.gamepads.entry(gamepad).or_default();
        if entry.pressed_buttons.insert(button) {
            entry.just_pressed_buttons.insert(button);
            self.queued_events
                .push(InputEvent::GamepadButtonPressed { gamepad, button });
        }
    }

    pub fn release_gamepad_button(&mut self, gamepad: GamepadId, button: GamepadButton) {
        let entry = self.gamepads.entry(gamepad).or_default();
        if entry.pressed_buttons.remove(&button) {
            entry.just_released_buttons.insert(button);
            self.queued_events
                .push(InputEvent::GamepadButtonReleased { gamepad, button });
        }
    }

    pub fn set_axis(&mut self, gamepad: GamepadId, axis: GamepadAxis, value: f32) {
        let value = value.clamp(-1.0, 1.0);
        let entry = self.gamepads.entry(gamepad).or_default();
        entry.axes.insert(axis, value);
        self.queued_events.push(InputEvent::GamepadAxisChanged {
            gamepad,
            axis,
            value,
        });
    }
}

impl InputBackend for MockInputBackend {
    fn begin_frame(&mut self) {
        self.just_pressed_keys.clear();
        self.just_released_keys.clear();
        for pad in self.gamepads.values_mut() {
            pad.just_pressed_buttons.clear();
            pad.just_released_buttons.clear();
        }
        self.mouse_delta = Vec2::ZERO;
    }

    fn poll(&mut self) -> Vec<InputEvent> {
        std::mem::take(&mut self.queued_events)
    }

    fn is_pressed(&self, key: KeyCode) -> bool {
        self.pressed_keys.contains(&key)
    }

    fn just_pressed(&self, key: KeyCode) -> bool {
        self.just_pressed_keys.contains(&key)
    }

    fn just_released(&self, key: KeyCode) -> bool {
        self.just_released_keys.contains(&key)
    }

    fn pressed_keys(&self) -> HashSet<KeyCode> {
        self.pressed_keys.clone()
    }

    fn is_mouse_pressed(&self, button: MouseButton) -> bool {
        self.pressed_mouse.contains(&button)
    }

    fn mouse_position(&self) -> Vec2 {
        self.mouse_position
    }

    fn mouse_delta(&self) -> Vec2 {
        self.mouse_delta
    }

    fn gamepads(&self) -> Vec<GamepadId> {
        self.gamepads.keys().copied().collect()
    }

    fn is_button_pressed(&self, gamepad: GamepadId, button: GamepadButton) -> bool {
        self.gamepads
            .get(&gamepad)
            .is_some_and(|state| state.pressed_buttons.contains(&button))
    }

    fn just_button_pressed(&self, gamepad: GamepadId, button: GamepadButton) -> bool {
        self.gamepads
            .get(&gamepad)
            .is_some_and(|state| state.just_pressed_buttons.contains(&button))
    }

    fn just_button_released(&self, gamepad: GamepadId, button: GamepadButton) -> bool {
        self.gamepads
            .get(&gamepad)
            .is_some_and(|state| state.just_released_buttons.contains(&button))
    }

    fn axis_value(&self, gamepad: GamepadId, axis: GamepadAxis) -> f32 {
        self.gamepads
            .get(&gamepad)
            .and_then(|state| state.axes.get(&axis).copied())
            .unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action_map::{ActionMap, InputBinding};

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum TestAction {
        Jump,
        MoveForward,
        Shoot,
    }

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

    #[test]
    fn action_map_is_pressed_via_key() {
        let mut map = ActionMap::<TestAction>::new();
        map.bind(TestAction::Jump, InputBinding::Key(KeyCode::Space));

        let mut backend = MockInputBackend::new();
        assert!(!map.is_pressed(TestAction::Jump, &backend));
        backend.press_key(KeyCode::Space);
        assert!(map.is_pressed(TestAction::Jump, &backend));
    }

    #[test]
    fn action_map_just_pressed_is_edge_triggered() {
        let mut map = ActionMap::<TestAction>::new();
        map.bind(TestAction::Jump, InputBinding::Key(KeyCode::Space));

        let mut backend = MockInputBackend::new();
        backend.press_key(KeyCode::Space);
        assert!(map.just_pressed(TestAction::Jump, &backend));
        backend.begin_frame();
        assert!(!map.just_pressed(TestAction::Jump, &backend));
        // Still held but no longer "just".
        assert!(map.is_pressed(TestAction::Jump, &backend));
    }

    #[test]
    fn action_map_combines_bindings() {
        let mut map = ActionMap::<TestAction>::new();
        map.bind(TestAction::Shoot, InputBinding::Key(KeyCode::ControlLeft));
        map.bind(TestAction::Shoot, InputBinding::Mouse(MouseButton::Left));

        let mut backend = MockInputBackend::new();
        assert!(!map.is_pressed(TestAction::Shoot, &backend));
        backend.press_mouse(MouseButton::Left);
        assert!(map.is_pressed(TestAction::Shoot, &backend));
        backend.release_mouse(MouseButton::Left);
        backend.press_key(KeyCode::ControlLeft);
        assert!(map.is_pressed(TestAction::Shoot, &backend));
    }

    #[test]
    fn action_map_axis_value_picks_strongest() {
        let mut map = ActionMap::<TestAction>::new();
        map.bind(TestAction::MoveForward, InputBinding::Key(KeyCode::KeyW));
        let id = unsafe { std::mem::zeroed::<GamepadId>() };
        map.bind(
            TestAction::MoveForward,
            InputBinding::GamepadAxis {
                gamepad: id,
                axis: GamepadAxis::LeftStickY,
                threshold: 0.0,
            },
        );

        let mut backend = MockInputBackend::new();
        backend.add_gamepad(id);

        // Axis at 0.6 wins over 0 from key.
        backend.set_axis(id, GamepadAxis::LeftStickY, 0.6);
        assert!((map.axis_value(TestAction::MoveForward, &backend) - 0.6).abs() < 1e-6);

        // Key pressed (1.0) wins over axis at 0.6.
        backend.press_key(KeyCode::KeyW);
        assert!((map.axis_value(TestAction::MoveForward, &backend) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn action_map_unbind_removes_action() {
        let mut map = ActionMap::<TestAction>::new();
        map.bind(TestAction::Jump, InputBinding::Key(KeyCode::Space));
        assert_eq!(map.bindings_for(TestAction::Jump).len(), 1);
        map.unbind(TestAction::Jump);
        assert!(map.bindings_for(TestAction::Jump).is_empty());
    }

    #[test]
    fn unbound_action_returns_false_and_zero() {
        let map = ActionMap::<TestAction>::new();
        let backend = MockInputBackend::new();
        assert!(!map.is_pressed(TestAction::Jump, &backend));
        assert!(!map.just_pressed(TestAction::Jump, &backend));
        assert_eq!(map.axis_value(TestAction::Jump, &backend), 0.0);
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
}
