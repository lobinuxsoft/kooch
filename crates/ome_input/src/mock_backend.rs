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
/// Polling drains queued events and clears `just_pressed` /
/// `just_released` so frame deltas stay sharp.
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
            self.queued_events
                .push(InputEvent::GamepadButtonPressed { gamepad, button });
        }
    }

    pub fn release_gamepad_button(&mut self, gamepad: GamepadId, button: GamepadButton) {
        let entry = self.gamepads.entry(gamepad).or_default();
        if entry.pressed_buttons.remove(&button) {
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
    fn poll(&mut self) -> Vec<InputEvent> {
        self.just_pressed_keys.clear();
        self.just_released_keys.clear();
        self.mouse_delta = Vec2::ZERO;
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
    fn just_pressed_clears_on_poll() {
        let mut backend = MockInputBackend::new();
        backend.press_key(KeyCode::KeyA);
        assert!(backend.just_pressed(KeyCode::KeyA));
        backend.poll();
        assert!(!backend.just_pressed(KeyCode::KeyA));
        // Still pressed though.
        assert!(backend.is_pressed(KeyCode::KeyA));
    }

    #[test]
    fn just_released_clears_on_poll() {
        let mut backend = MockInputBackend::new();
        backend.press_key(KeyCode::KeyB);
        backend.poll();
        backend.release_key(KeyCode::KeyB);
        assert!(backend.just_released(KeyCode::KeyB));
        backend.poll();
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
        backend.poll();
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
        backend.poll();
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
        map.bind(
            TestAction::MoveForward,
            InputBinding::Key(KeyCode::KeyW),
        );
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
}
