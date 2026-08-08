//! Input for a process that has no window.
//!
//! The editor's Play button runs the project as a **headless host**: it
//! simulates, the editor draws. Headless means no window, and no window
//! means no `WindowEvent` — so the host never sees a key. Pressing Play
//! and then a key did nothing at all (#710).
//!
//! [`RemoteInputBackend`] closes that: the editor captures input from its
//! own window and sends [`InputSnapshot`]s over the protocol, and this
//! applies them. Gameplay reads `Box<dyn InputBackend>` and cannot tell
//! which process filled it — the same code runs in the shipped game,
//! where `WinitGilrsBackend` fills it directly.
//!
//! # State, not events
//!
//! A snapshot says *what is held*, never *what changed*. Three reasons,
//! and the first is the one that matters:
//!
//! - **A dropped frame cannot leave a key stuck down.** With events, one
//!   lost `KeyReleased` means the player walks into a wall forever. With
//!   state, the next snapshot corrects it.
//! - It is idempotent, so a resend is free.
//! - The edges (`just_pressed`, `just_released`) are derived here by
//!   comparing consecutive snapshots, which is the same thing the local
//!   backend does — one definition of "this frame", not two.

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

/// Everything an input backend holds, at one instant.
///
/// Sorted on the way out (see [`InputSnapshot::from_backend`]) so two
/// equal states serialise identically — which is what lets the sender
/// skip a send when nothing changed.
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
    /// Cursor movement accumulated over the frame being described.
    ///
    /// A delta rather than a difference of positions: the sender resets
    /// it every frame, and two positions cannot tell a still cursor from
    /// one that went out and came back.
    #[serde(default)]
    pub mouse_delta: [f32; 2],
    /// Connected gamepads and their state.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gamepads: Vec<GamepadSnapshot>,
}

impl InputSnapshot {
    /// Reads the current state of any backend into a snapshot.
    ///
    /// Ordering is fixed rather than whatever the backend's sets iterate
    /// in, so an unchanged state produces an equal snapshot and the
    /// sender can tell "nothing happened" from "something did".
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
        Self {
            keys,
            mouse_buttons,
            mouse_position: [position.x, position.y],
            mouse_delta: [delta.x, delta.y],
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
    gamepads: HashMap<GamepadId, PadState>,
    /// Applied but not yet handed out by `poll`.
    queued_events: Vec<InputEvent>,
}

#[derive(Default)]
struct PadState {
    /// Edges derived in `apply`, from the difference between two state
    /// snapshots. Expired there too, for the same reason the key edges
    /// are: the host ticks faster than the editor sends, so clearing on
    /// `begin_frame` would drop a press before anything read it.
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

    /// Replaces the held state with `snapshot`, deriving the edges from
    /// what was held before.
    ///
    /// **This is what expires the previous edges**, not `begin_frame` —
    /// see the note there.
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

    /// Releases everything, as though every device were let go.
    ///
    /// What Stop calls. Without it, a key held at the moment play ended
    /// stays held in this backend, and the next play session starts with
    /// the player already walking.
    pub fn release_all(&mut self) {
        let snapshot = InputSnapshot::default();
        self.apply(&snapshot);
    }
}

impl InputBackend for RemoteInputBackend {
    fn apply_snapshot(&mut self, snapshot: &InputSnapshot) {
        self.apply(snapshot);
    }

    /// Deliberately does nothing.
    ///
    /// # Why the frame boundary is not here
    ///
    /// For a local backend a frame is the unit an edge lives for, and
    /// `begin_frame` is what ends it. Here the two processes do not tick
    /// together: the host runs its own loop and the editor sends at its
    /// own rate, usually slower. Expiring edges on the host's frame means
    /// a keypress that arrives between two host frames is cleared before
    /// any system reads it — the #711 bug again, one process over.
    ///
    /// So a snapshot *is* the frame boundary, and [`apply`](Self::apply)
    /// is what expires the previous one. An edge lives from the snapshot
    /// that produced it until the snapshot that supersedes it, however
    /// many host frames that spans.
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
