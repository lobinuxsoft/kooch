//! [`InputBackend`] and shared types — winit+gilrs, remote and mock backends implement it.
//! Identifiers are the engine's own ([`crate::ids`]), since a remote host, saved bindings and Steam
//! Input all need them.

use glam::Vec2;
use std::collections::HashSet;

pub use crate::ids::{GamepadAxis, GamepadButton, GamepadId, KeyCode, MouseButton};

/// Per-frame input event from [`InputBackend::poll`], for systems needing edges; gameplay usually
/// reads the cumulative state.
#[derive(Debug, Clone, Copy)]
pub enum InputEvent {
    KeyPressed(KeyCode),
    KeyReleased(KeyCode),
    MousePressed(MouseButton),
    MouseReleased(MouseButton),
    MouseMoved {
        position: Vec2,
        delta: Vec2,
    },
    GamepadConnected(GamepadId),
    GamepadDisconnected(GamepadId),
    GamepadButtonPressed {
        gamepad: GamepadId,
        button: GamepadButton,
    },
    GamepadButtonReleased {
        gamepad: GamepadId,
        button: GamepadButton,
    },
    GamepadAxisChanged {
        gamepad: GamepadId,
        axis: GamepadAxis,
        value: f32,
    },
}

/// Engine input interface: backends keep the held state, updated in [`poll`](Self::poll) and by fed
/// events, and expose it read-only.
///
/// ```text
/// per frame, in this order:
///   1. backend.begin_frame()   → forgets last frame's edges
///   2. this frame's raw events are fed in (winit → feed_window_event)
///   3. backend.poll()          → drains device sources, returns events
///   4. game systems read is_pressed / just_pressed / mouse_delta / …
/// ```
///
/// [`InputPlugin`](crate::InputPlugin) runs 1–3 in `Stage::Input`. Clearing is its own call because
/// it must precede the frame's events — inside `poll` it wiped presses already delivered.
pub trait InputBackend: Send + Sync + 'static {
    /// Forgets the previous frame's `just_pressed` / `just_released`
    /// edges and mouse delta. Call once per frame, before feeding this
    /// frame's events.
    fn begin_frame(&mut self);

    /// Pushes one window event into the backend — on the trait because the engine holds a `dyn`;
    /// device-fed backends ignore it. Typed as winit's event, not `&dyn Any`.
    fn feed_window_event(&mut self, _event: &winit::event::WindowEvent) {}

    /// Replaces the held state with one captured elsewhere — the wire counterpart of
    /// [`feed_window_event`](Self::feed_window_event); device backends ignore it.
    fn apply_snapshot(&mut self, _snapshot: &crate::remote_backend::InputSnapshot) {}

    /// Drains pending events from device sources the backend polls
    /// itself (gamepads), and returns everything queued since the last
    /// call.
    fn poll(&mut self) -> Vec<InputEvent>;

    // ─── keyboard ────────────────────────────────────────────────────
    fn is_pressed(&self, key: KeyCode) -> bool;
    fn just_pressed(&self, key: KeyCode) -> bool;
    fn just_released(&self, key: KeyCode) -> bool;

    /// Snapshot of every key currently held. Useful for input rebind UI
    /// ("press any key").
    fn pressed_keys(&self) -> HashSet<KeyCode>;

    // ─── mouse ───────────────────────────────────────────────────────
    fn is_mouse_pressed(&self, button: MouseButton) -> bool;
    fn mouse_position(&self) -> Vec2;
    /// Cumulative delta since the previous `poll` call.
    fn mouse_delta(&self) -> Vec2;

    // ─── gamepad ─────────────────────────────────────────────────────
    fn gamepads(&self) -> Vec<GamepadId>;
    fn is_button_pressed(&self, gamepad: GamepadId, button: GamepadButton) -> bool;
    /// `true` only on the frame the button went down — without it a pad jump fired every held frame
    /// (#57).
    fn just_button_pressed(&self, gamepad: GamepadId, button: GamepadButton) -> bool;
    /// `true` only on the frame the button came back up.
    fn just_button_released(&self, gamepad: GamepadId, button: GamepadButton) -> bool;
    /// Returns the axis value in `[-1.0, 1.0]`, or `0.0` if the gamepad
    /// is disconnected / axis is unknown.
    fn axis_value(&self, gamepad: GamepadId, axis: GamepadAxis) -> f32;
}
