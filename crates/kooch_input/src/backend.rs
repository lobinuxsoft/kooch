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
    /// The wheel or a trackpad scrolled, in lines: `+y` away from the user, `+x` right (#56).
    MouseScrolled {
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
    /// The cursor as a pointer, in view pixels: `None` while it is captured for mouse look or out
    /// of the view. What UI picking reads (#96).
    fn pointer(&self) -> Option<Vec2> {
        Some(self.mouse_position())
    }
    /// Whether the cursor is held for mouse look. Told by whoever applies the capture.
    fn capture_pointer(&mut self, _captured: bool) {}
    /// Hands this frame's mouse buttons and scroll to the UI: actions then read them as idle, so a
    /// click on a button does not also fire the game's (#96). Raw reads are unaffected.
    fn claim_pointer(&mut self, _claimed: bool) {}
    fn pointer_claimed(&self) -> bool {
        false
    }
    /// Cumulative delta since the previous `poll` call.
    fn mouse_delta(&self) -> Vec2;
    /// Pushes relative mouse motion — the device's own report, which does not stop at the edge of
    /// the window the way two cursor positions do (#1266). Backends with no mouse ignore it.
    fn feed_mouse_motion(&mut self, _delta: Vec2) {}
    /// The mouse's motion over the last frame as a velocity, in pixels per second. Zero where the
    /// backend has no raw motion: a rate made up from cursor positions would stop at the edge.
    fn mouse_velocity(&self) -> Vec2 {
        Vec2::ZERO
    }
    /// Scroll since the previous frame, in lines: `+y` away from the user, `+x` right. A
    /// trackpad's pixels are converted, so a wheel notch and a swipe read alike (#56).
    fn mouse_scroll(&self) -> Vec2 {
        Vec2::ZERO
    }

    // ─── gamepad ─────────────────────────────────────────────────────
    /// Connected pads, lowest id first.
    fn gamepads(&self) -> Vec<GamepadId>;
    /// The pad actions read: the last one touched, or the lowest id before any is (#1643).
    fn active_gamepad(&self) -> Option<GamepadId> {
        self.gamepads().first().copied()
    }
    fn is_button_pressed(&self, gamepad: GamepadId, button: GamepadButton) -> bool;
    /// `true` only on the frame the button went down — without it a pad jump fired every held frame
    /// (#57).
    fn just_button_pressed(&self, gamepad: GamepadId, button: GamepadButton) -> bool;
    /// `true` only on the frame the button came back up.
    fn just_button_released(&self, gamepad: GamepadId, button: GamepadButton) -> bool;
    /// Returns the axis value in `[-1.0, 1.0]`, or `0.0` if the gamepad
    /// is disconnected / axis is unknown.
    fn axis_value(&self, gamepad: GamepadId, axis: GamepadAxis) -> f32;
    /// How far a button is pulled, in `[0, 1]`: a trigger's travel, 0 or 1 for a digital button
    /// (#57). Defaults to the held state for backends that only know held.
    fn button_value(&self, gamepad: GamepadId, button: GamepadButton) -> f32 {
        self.is_button_pressed(gamepad, button) as u8 as f32
    }
    /// Plays a rumble on a pad, replacing the one it was playing. A pad without force feedback, or a
    /// backend without pads, ignores it.
    fn rumble(&mut self, _gamepad: GamepadId, _rumble: crate::rumble::Rumble) {}
    /// Rumbles the pad last used, if any.
    fn rumble_active(&mut self, rumble: crate::rumble::Rumble) {
        if let Some(pad) = self.active_gamepad() {
            self.rumble(pad, rumble);
        }
    }
    /// Rumbles this backend cannot play itself, for whoever holds the pad: the editor, in Play.
    fn take_rumbles(&mut self) -> Vec<(GamepadId, crate::rumble::Rumble)> {
        Vec::new()
    }
}
