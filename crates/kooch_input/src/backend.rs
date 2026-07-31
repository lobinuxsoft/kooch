//! [`InputBackend`] trait + shared types.
//!
//! Backends (`WinitGilrsBackend`, `MockInputBackend`, future SDL2 / Steam
//! Input) implement the trait. Game code calls trait methods via
//! `Box<dyn InputBackend>` stored as a [`Resource`](kooch_core::resource::Resources).
//!
//! Re-exports `winit::keyboard::KeyCode` + `winit::event::MouseButton` +
//! `gilrs::{Button, Axis, GamepadId}` directly — engine-neutral wrapper
//! types are deferred until a non-winit backend ships (which is unlikely
//! in the foreseeable roadmap). Less surface area, less mapping code.

use glam::Vec2;
use std::collections::HashSet;

pub use gilrs::{Axis as GamepadAxis, Button as GamepadButton, GamepadId};
pub use winit::event::MouseButton;
pub use winit::keyboard::KeyCode;

/// Per-frame input event surfaced by [`InputBackend::poll`].
///
/// Game code typically reads cumulative state (`is_pressed`, etc.) rather
/// than processing this event stream — events exist for systems that need
/// edge detection (UI focus, char input, gesture recognizers).
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

/// Engine input interface.
///
/// Backends maintain internal state (currently-pressed keys, mouse pos,
/// etc.) and expose it via the read-only methods. Mutating frame state
/// happens inside [`poll`](Self::poll) and event-feeding methods specific
/// to each backend (e.g. `WinitGilrsBackend::feed_window_event`).
///
/// # Frame lifecycle
///
/// ```text
/// per frame:
///   1. event sources (winit, gilrs) push raw events to the backend
///   2. game systems call backend.poll() once  →  Vec<InputEvent>
///   3. game systems read backend.is_pressed / mouse_delta / etc.
/// ```
///
/// `just_pressed` / `just_released` reflect transitions during the
/// frame *between consecutive `poll` calls*. They are cleared by `poll`.
pub trait InputBackend: Send + Sync + 'static {
    /// Drains pending events. Clears `just_pressed` / `just_released`
    /// state so the next frame's deltas are fresh.
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
    /// Returns the axis value in `[-1.0, 1.0]`, or `0.0` if the gamepad
    /// is disconnected / axis is unknown.
    fn axis_value(&self, gamepad: GamepadId, axis: GamepadAxis) -> f32;
}
