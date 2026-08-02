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
/// per frame, in this order:
///   1. backend.begin_frame()   → forgets last frame's edges
///   2. this frame's raw events are fed in (winit → feed_window_event)
///   3. backend.poll()          → drains device sources, returns events
///   4. game systems read is_pressed / just_pressed / mouse_delta / …
/// ```
///
/// [`InputPlugin`](crate::InputPlugin) runs 1–3 in `Stage::Input`, so a
/// gameplay system in `Stage::Update` sees this frame's input.
///
/// # Why clearing has its own call
///
/// `just_pressed` is true for exactly one frame, which only works if the
/// clear happens *before* the frame's events are applied and never
/// after. It used to live at the top of `poll`, where it deleted the
/// presses winit had already delivered — a key pressed between two
/// frames was recorded and then wiped before any system could read it,
/// so `just_pressed` was permanently false. Nothing caught it because
/// nothing called any of this.
pub trait InputBackend: Send + Sync + 'static {
    /// Forgets the previous frame's `just_pressed` / `just_released`
    /// edges and mouse delta. Call once per frame, before feeding this
    /// frame's events.
    fn begin_frame(&mut self);

    /// Pushes one window event into the backend.
    ///
    /// On the trait rather than on the concrete backend because the
    /// engine holds a `Box<dyn InputBackend>` and the window runner has
    /// no idea which one it is. Backends fed entirely by their own
    /// device sources — a Steam Input backend, a replay backend — take
    /// the default and ignore it.
    ///
    /// Typed as winit's event rather than `&dyn Any` because this crate
    /// re-exports winit's `KeyCode` and `MouseButton` already: hiding
    /// the one type it takes behind a downcast buys no independence and
    /// costs a runtime failure mode.
    fn feed_window_event(&mut self, _event: &winit::event::WindowEvent) {}

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
    /// Returns the axis value in `[-1.0, 1.0]`, or `0.0` if the gamepad
    /// is disconnected / axis is unknown.
    fn axis_value(&self, gamepad: GamepadId, axis: GamepadAxis) -> f32;
}
