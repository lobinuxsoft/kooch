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
mod tests;
