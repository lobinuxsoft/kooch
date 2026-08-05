//! [`WinitGilrsBackend`] — production [`InputBackend`] backed by winit + gilrs.
//!
//! winit pushes events at us via `feed_window_event`; gilrs is poll-based
//! so we drain its event queue inside `poll`.

use glam::Vec2;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use gilrs::Gilrs;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::keyboard::PhysicalKey;

use crate::backend::{
    GamepadAxis, GamepadButton, GamepadId, InputBackend, InputEvent, KeyCode, MouseButton,
};

/// Winit + gilrs powered backend. Stores per-frame state, accumulates
/// events in [`feed_window_event`](InputBackend::feed_window_event) and
/// [`poll`](InputBackend::poll), and exposes immediate state via the
/// [`InputBackend`] trait.
///
/// `gilrs::Gilrs` is `!Sync` (internal mutability for the device cache)
/// so we wrap it in `Mutex` to satisfy the trait's `Sync` bound. Lock
/// contention is irrelevant — only the main-thread `poll` ever touches
/// it.
pub struct WinitGilrsBackend {
    /// `None` when gilrs could not enumerate a device backend.
    ///
    /// Gamepads are optional; a keyboard is not. Failing construction
    /// over gilrs would have left a headless-ish Linux box — no evdev
    /// access, a container, a bare Wayland session — with **no input at
    /// all**, when the thing that failed drives none of the keys.
    gilrs: Option<Mutex<Gilrs>>,
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
    /// Buttons that went down this frame, cleared by `begin_frame` — the
    /// same shape as `just_pressed_keys`.
    just_pressed_buttons: HashSet<GamepadButton>,
    just_released_buttons: HashSet<GamepadButton>,
    axes: HashMap<GamepadAxis, f32>,
}

impl WinitGilrsBackend {
    /// Creates a backend, initialising the gilrs context.
    ///
    /// Gamepad support degrades to nothing if gilrs cannot enumerate a
    /// device backend (on Linux: headless, no evdev access, a container).
    /// Keyboard and mouse are unaffected — they arrive from winit.
    pub fn new() -> Self {
        let gilrs = match Gilrs::new() {
            Ok(gilrs) => Some(Mutex::new(gilrs)),
            Err(error) => {
                tracing::warn!(%error, "gamepad support unavailable; keyboard and mouse still work");
                None
            }
        };
        Self {
            gilrs,
            pressed_keys: HashSet::new(),
            just_pressed_keys: HashSet::new(),
            just_released_keys: HashSet::new(),
            pressed_mouse: HashSet::new(),
            mouse_position: Vec2::ZERO,
            mouse_delta: Vec2::ZERO,
            gamepads: HashMap::new(),
            queued_events: Vec::new(),
        }
    }

    fn apply_window_event(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key,
                        state,
                        repeat,
                        ..
                    },
                ..
            } => {
                if *repeat {
                    return; // ignore key repeats — we track press/release transitions
                }
                let PhysicalKey::Code(code) = physical_key else {
                    return;
                };
                // A key this engine has no name for is a key no binding
                // can mention, so tracking it would only grow the set.
                let Some(key) = KeyCode::from_upstream(*code) else {
                    return;
                };
                let key = &key;
                match state {
                    ElementState::Pressed => {
                        if self.pressed_keys.insert(*key) {
                            self.just_pressed_keys.insert(*key);
                            self.queued_events.push(InputEvent::KeyPressed(*key));
                        }
                    }
                    ElementState::Released => {
                        if self.pressed_keys.remove(key) {
                            self.just_released_keys.insert(*key);
                            self.queued_events.push(InputEvent::KeyReleased(*key));
                        }
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let Some(button) = MouseButton::from_upstream(*button) else {
                    return;
                };
                match state {
                    ElementState::Pressed => {
                        if self.pressed_mouse.insert(button) {
                            self.queued_events.push(InputEvent::MousePressed(button));
                        }
                    }
                    ElementState::Released => {
                        if self.pressed_mouse.remove(&button) {
                            self.queued_events.push(InputEvent::MouseReleased(button));
                        }
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let new_pos = Vec2::new(position.x as f32, position.y as f32);
                let delta = new_pos - self.mouse_position;
                self.mouse_position = new_pos;
                self.mouse_delta += delta;
                self.queued_events.push(InputEvent::MouseMoved {
                    position: new_pos,
                    delta,
                });
            }
            _ => {}
        }
    }

    fn drain_gilrs(&mut self) {
        let Some(gilrs) = &self.gilrs else {
            return;
        };
        let mut gilrs = gilrs.lock().expect("gilrs mutex poisoned");
        while let Some(gilrs::Event { id, event, .. }) = gilrs.next_event() {
            let id = GamepadId::from(id);
            match event {
                gilrs::EventType::Connected => {
                    self.gamepads.entry(id).or_default();
                    self.queued_events.push(InputEvent::GamepadConnected(id));
                }
                gilrs::EventType::Disconnected => {
                    self.gamepads.remove(&id);
                    self.queued_events.push(InputEvent::GamepadDisconnected(id));
                }
                gilrs::EventType::ButtonPressed(button, _) => {
                    let Some(button) = GamepadButton::from_upstream(button) else {
                        continue;
                    };
                    let entry = self.gamepads.entry(id).or_default();
                    if entry.pressed_buttons.insert(button) {
                        entry.just_pressed_buttons.insert(button);
                        self.queued_events.push(InputEvent::GamepadButtonPressed {
                            gamepad: id,
                            button,
                        });
                    }
                }
                gilrs::EventType::ButtonReleased(button, _) => {
                    let Some(button) = GamepadButton::from_upstream(button) else {
                        continue;
                    };
                    let entry = self.gamepads.entry(id).or_default();
                    if entry.pressed_buttons.remove(&button) {
                        entry.just_released_buttons.insert(button);
                        self.queued_events.push(InputEvent::GamepadButtonReleased {
                            gamepad: id,
                            button,
                        });
                    }
                }
                gilrs::EventType::AxisChanged(axis, value, _) => {
                    let Some(axis) = GamepadAxis::from_upstream(axis) else {
                        continue;
                    };
                    let entry = self.gamepads.entry(id).or_default();
                    entry.axes.insert(axis, value);
                    self.queued_events.push(InputEvent::GamepadAxisChanged {
                        gamepad: id,
                        axis,
                        value,
                    });
                }
                _ => {} // ignore button-changed analog events for PR-1
            }
        }
    }
}

impl InputBackend for WinitGilrsBackend {
    fn begin_frame(&mut self) {
        self.just_pressed_keys.clear();
        for pad in self.gamepads.values_mut() {
            pad.just_pressed_buttons.clear();
            pad.just_released_buttons.clear();
        }
        self.just_released_keys.clear();
        self.mouse_delta = Vec2::ZERO;
    }

    fn feed_window_event(&mut self, event: &WindowEvent) {
        self.apply_window_event(event);
    }

    fn poll(&mut self) -> Vec<InputEvent> {
        self.drain_gilrs();
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
