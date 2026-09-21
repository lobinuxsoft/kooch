//! Input for the windowless host behind Play (#710): the editor sends [`InputSnapshot`]s, applied
//! here behind the same `Box<dyn InputBackend>`.
//! State, not events, so a dropped snapshot cannot leave a key stuck.

use std::collections::{HashMap, HashSet};

use glam::Vec2;
use serde::{Deserialize, Serialize};

use crate::backend::{InputBackend, InputEvent};
use crate::ids::{GamepadAxis, GamepadButton, GamepadId, KeyCode, MouseButton};

/// One gamepad's state at an instant.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GamepadSnapshot {
    /// Which pad, as the sending backend numbers them.
    pub id: u32,
    /// Buttons currently held.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buttons: Vec<GamepadButton>,
    /// Axes and their values. Absent axes read as `0.0`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub axes: Vec<(GamepadAxis, f32)>,
}

/// Everything an input backend holds at one instant, sorted so equal states serialise equally and
/// an unchanged frame can skip sending.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InputSnapshot {
    /// Keys currently held.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<KeyCode>,
    /// Mouse buttons currently held.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mouse_buttons: Vec<MouseButton>,
    /// Cursor position, in the coordinate space of whatever captured it.
    #[serde(default)]
    pub mouse_position: [f32; 2],
    /// Cursor movement accumulated over the frame, as a delta — two positions cannot tell a still
    /// cursor from a round trip.
    #[serde(default)]
    pub mouse_delta: [f32; 2],
    /// The mouse's raw motion as pixels per second (#1266). A rate rather than a delta, because the
    /// host does not tick with the editor: a delta applied once per host frame is counted twice, or
    /// not at all, while a velocity is right whenever it is read.
    #[serde(default)]
    pub mouse_velocity: [f32; 2],
    /// Connected gamepads and their state.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gamepads: Vec<GamepadSnapshot>,
}

impl InputSnapshot {
    /// Reads any backend into a snapshot, in fixed order so an unchanged state compares equal.
    pub fn from_backend(backend: &dyn InputBackend) -> Self {
        let mut keys: Vec<KeyCode> = backend.pressed_keys().into_iter().collect();
        keys.sort_unstable();

        let mouse_buttons = [
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::Back,
            MouseButton::Forward,
        ]
        .into_iter()
        .filter(|button| backend.is_mouse_pressed(*button))
        .collect();

        const BUTTONS: [GamepadButton; 19] = [
            GamepadButton::South,
            GamepadButton::East,
            GamepadButton::North,
            GamepadButton::West,
            GamepadButton::C,
            GamepadButton::Z,
            GamepadButton::LeftTrigger,
            GamepadButton::LeftTrigger2,
            GamepadButton::RightTrigger,
            GamepadButton::RightTrigger2,
            GamepadButton::Select,
            GamepadButton::Start,
            GamepadButton::Mode,
            GamepadButton::LeftThumb,
            GamepadButton::RightThumb,
            GamepadButton::DPadUp,
            GamepadButton::DPadDown,
            GamepadButton::DPadLeft,
            GamepadButton::DPadRight,
        ];
        const AXES: [GamepadAxis; 8] = [
            GamepadAxis::LeftStickX,
            GamepadAxis::LeftStickY,
            GamepadAxis::LeftZ,
            GamepadAxis::RightStickX,
            GamepadAxis::RightStickY,
            GamepadAxis::RightZ,
            GamepadAxis::DPadX,
            GamepadAxis::DPadY,
        ];

        let mut gamepads: Vec<GamepadSnapshot> = backend
            .gamepads()
            .into_iter()
            .map(|pad| GamepadSnapshot {
                id: pad.index(),
                buttons: BUTTONS
                    .into_iter()
                    .filter(|button| backend.is_button_pressed(pad, *button))
                    .collect(),
                // Only axes that are off centre: a resting pad reports
                // eight zeroes otherwise, every frame, forever.
                axes: AXES
                    .into_iter()
                    .map(|axis| (axis, backend.axis_value(pad, axis)))
                    .filter(|(_, value)| *value != 0.0)
                    .collect(),
            })
            .collect();
        gamepads.sort_unstable_by_key(|pad| pad.id);

        let position = backend.mouse_position();
        let delta = backend.mouse_delta();
        let velocity = backend.mouse_velocity();
        Self {
            keys,
            mouse_buttons,
            mouse_position: [position.x, position.y],
            mouse_delta: [delta.x, delta.y],
            mouse_velocity: [velocity.x, velocity.y],
            gamepads,
        }
    }

    /// Whether this describes a state nothing is pressed or moving in.
    ///
    /// What the sender checks before deciding a send is worth a frame.
    pub fn is_idle(&self) -> bool {
        self.keys.is_empty()
            && self.mouse_buttons.is_empty()
            && self.mouse_delta == [0.0, 0.0]
            // A mouse that stopped has to say so once, or the host keeps turning at the last speed.
            && self.mouse_velocity == [0.0, 0.0]
            && self.gamepads.iter().all(|pad| {
                pad.buttons.is_empty() && pad.axes.iter().all(|(_, value)| *value == 0.0)
            })
    }
}

/// An [`InputBackend`] fed by [`InputSnapshot`]s instead of by devices.
#[derive(Default)]
pub struct RemoteInputBackend {
    pressed_keys: HashSet<KeyCode>,
    just_pressed_keys: HashSet<KeyCode>,
    just_released_keys: HashSet<KeyCode>,
    pressed_mouse: HashSet<MouseButton>,
    mouse_position: Vec2,
    mouse_delta: Vec2,
    /// Held until the next snapshot, like an axis: the host ticks faster than the editor sends.
    mouse_velocity: Vec2,
    gamepads: HashMap<GamepadId, PadState>,
    /// Applied but not yet handed out by `poll`.
    queued_events: Vec<InputEvent>,
}

#[derive(Default)]
struct PadState {
    /// Edges derived in `apply` and expired there too, not in `begin_frame`, since the host ticks
    /// faster than the editor sends.
    just_pressed: HashSet<GamepadButton>,
    just_released: HashSet<GamepadButton>,
    buttons: HashSet<GamepadButton>,
    axes: HashMap<GamepadAxis, f32>,
}

impl RemoteInputBackend {
    /// Creates an empty backend: nothing pressed, no pads.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the held state with `snapshot`, deriving edges — **this** expires the previous
    /// ones, not `begin_frame`.
    pub fn apply(&mut self, snapshot: &InputSnapshot) {
        self.just_pressed_keys.clear();
        self.just_released_keys.clear();
        self.mouse_delta = Vec2::ZERO;

        let incoming: HashSet<KeyCode> = snapshot.keys.iter().copied().collect();
        for &key in incoming.difference(&self.pressed_keys) {
            self.just_pressed_keys.insert(key);
            self.queued_events.push(InputEvent::KeyPressed(key));
        }
        for &key in self.pressed_keys.difference(&incoming) {
            self.just_released_keys.insert(key);
            self.queued_events.push(InputEvent::KeyReleased(key));
        }
        self.pressed_keys = incoming;

        let incoming: HashSet<MouseButton> = snapshot.mouse_buttons.iter().copied().collect();
        for &button in incoming.difference(&self.pressed_mouse) {
            self.queued_events.push(InputEvent::MousePressed(button));
        }
        for &button in self.pressed_mouse.difference(&incoming) {
            self.queued_events.push(InputEvent::MouseReleased(button));
        }
        self.pressed_mouse = incoming;

        let position = Vec2::from(snapshot.mouse_position);
        let delta = Vec2::from(snapshot.mouse_delta);
        self.mouse_position = position;
        self.mouse_delta = delta;
        self.mouse_velocity = Vec2::from(snapshot.mouse_velocity);
        if delta != Vec2::ZERO {
            self.queued_events
                .push(InputEvent::MouseMoved { position, delta });
        }

        let mut seen = HashSet::new();
        for pad in &snapshot.gamepads {
            let id = GamepadId(pad.id);
            seen.insert(id);
            if !self.gamepads.contains_key(&id) {
                self.queued_events.push(InputEvent::GamepadConnected(id));
            }
            let state = self.gamepads.entry(id).or_default();
            let incoming: HashSet<GamepadButton> = pad.buttons.iter().copied().collect();
            state.just_pressed.clear();
            state.just_released.clear();
            for &button in incoming.difference(&state.buttons) {
                state.just_pressed.insert(button);
                self.queued_events.push(InputEvent::GamepadButtonPressed {
                    gamepad: id,
                    button,
                });
            }
            for &button in state.buttons.difference(&incoming) {
                state.just_released.insert(button);
                self.queued_events.push(InputEvent::GamepadButtonReleased {
                    gamepad: id,
                    button,
                });
            }
            state.buttons = incoming;
            // Replaced, not merged: an axis the snapshot omits is centred,
            // and merging would leave a stick pushed forever after the
            // sender stopped mentioning it.
            state.axes = pad.axes.iter().copied().collect();
            for &(axis, value) in &pad.axes {
                self.queued_events.push(InputEvent::GamepadAxisChanged {
                    gamepad: id,
                    axis,
                    value,
                });
            }
        }
        let gone: Vec<GamepadId> = self
            .gamepads
            .keys()
            .copied()
            .filter(|id| !seen.contains(id))
            .collect();
        for id in gone {
            self.gamepads.remove(&id);
            self.queued_events.push(InputEvent::GamepadDisconnected(id));
        }
    }
}

impl InputBackend for RemoteInputBackend {
    fn apply_snapshot(&mut self, snapshot: &InputSnapshot) {
        self.apply(snapshot);
    }

    /// Deliberately does nothing: the processes tick independently, so a snapshot is the frame
    /// boundary, expired by [`apply`](Self::apply), not by the host's frame (#711).
    fn begin_frame(&mut self) {}

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

    fn mouse_velocity(&self) -> Vec2 {
        self.mouse_velocity
    }

    fn gamepads(&self) -> Vec<GamepadId> {
        let mut ids: Vec<GamepadId> = self.gamepads.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    fn is_button_pressed(&self, gamepad: GamepadId, button: GamepadButton) -> bool {
        self.gamepads
            .get(&gamepad)
            .is_some_and(|state| state.buttons.contains(&button))
    }

    fn just_button_pressed(&self, gamepad: GamepadId, button: GamepadButton) -> bool {
        self.gamepads
            .get(&gamepad)
            .is_some_and(|state| state.just_pressed.contains(&button))
    }

    fn just_button_released(&self, gamepad: GamepadId, button: GamepadButton) -> bool {
        self.gamepads
            .get(&gamepad)
            .is_some_and(|state| state.just_released.contains(&button))
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

#[cfg(test)]
mod repeat_tests;
