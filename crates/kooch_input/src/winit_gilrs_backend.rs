//! [`WinitGilrsBackend`] — the production [`InputBackend`]: winit pushes events, gilrs is drained
//! in `poll`.

use glam::Vec2;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Duration;

use gilrs::Gilrs;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::keyboard::PhysicalKey;

use crate::backend::{
    GamepadAxis, GamepadButton, GamepadId, InputBackend, InputEvent, KeyCode, MouseButton,
};

/// Winit + gilrs backend. `gilrs::Gilrs` is `!Sync`, so it sits in a `Mutex`; only the main-thread
/// `poll` touches it.
pub struct WinitGilrsBackend {
    /// `None` when gilrs could not enumerate a device backend — gamepads are optional, and keyboard
    /// input must not depend on them.
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

/// How long startup waits for gamepad enumeration: generous, since working enumeration takes
/// milliseconds.
const GILRS_TIMEOUT: Duration = Duration::from_secs(3);

/// Builds the gilrs context, giving up if it does not answer. 🔴 Under Proton on a OneXFly
/// enumeration never returns, and input is built before the window (#963).
/// ⚠️ The thread is never joined — joining a wedged driver would restore the hang.
fn init_gilrs(timeout: Duration) -> Option<Mutex<Gilrs>> {
    match build_within(timeout, || Gilrs::new().map_err(|error| error.to_string())) {
        Built::Ready(gilrs) => Some(Mutex::new(gilrs)),
        Built::Failed(error) => {
            tracing::warn!(%error, "gamepad support unavailable; keyboard and mouse still work");
            None
        }
        Built::NoAnswer => {
            tracing::warn!(
                timeout_secs = timeout.as_secs(),
                "gamepad enumeration did not answer; continuing without gamepads — \
                 keyboard and mouse still work",
            );
            None
        }
    }
}

/// What came back from a build that was given a deadline.
enum Built<T> {
    Ready(T),
    /// It answered, and the answer was no.
    Failed(String),
    /// It did not answer. Distinct from `Failed` because the causes and
    /// the messages differ: one is a backend that is absent, the other
    /// is a backend that is stuck.
    NoAnswer,
}

/// Runs `build` on its own thread and gives up after `timeout` — split out so the deadline tests
/// without a wedged driver.
fn build_within<T, F>(timeout: Duration, build: F) -> Built<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    if std::thread::Builder::new()
        .name("gilrs-init".into())
        .spawn(move || {
            // The receiver is gone once the deadline passes; a failed
            // send means nobody is waiting any more, not an error.
            let _ = tx.send(build());
        })
        .is_err()
    {
        return Built::Failed("could not start the initialisation thread".to_owned());
    }

    match rx.recv_timeout(timeout) {
        Ok(Ok(value)) => Built::Ready(value),
        Ok(Err(error)) => Built::Failed(error),
        Err(_) => Built::NoAnswer,
    }
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
    /// Creates a backend; gamepad support degrades to nothing if gilrs fails or never answers (see
    /// `init_gilrs`), keyboard and mouse unaffected.
    pub fn new() -> Self {
        Self {
            gilrs: init_gilrs(GILRS_TIMEOUT),
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

#[cfg(test)]
mod backend_tests;
