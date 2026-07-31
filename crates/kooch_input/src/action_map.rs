//! [`ActionMap`] — typed actions ↔ input bindings.
//!
//! Game code defines an enum of actions (`Jump`, `Shoot`, `MoveForward`...)
//! and binds each to one or more [`InputBinding`]s. Per frame:
//!
//! ```ignore
//! if action_map.just_pressed(Action::Jump, &*backend) {
//!     velocity.y = 5.0;
//! }
//! let forward = action_map.axis_value(Action::MoveForward, &*backend);
//! ```
//!
//! Multiple bindings per action let `WASD + left stick` map to the same
//! `MoveForward` action — the strongest signal wins for axes, ANY pressed
//! wins for booleans.

use std::collections::HashMap;
use std::hash::Hash;

use crate::backend::{
    GamepadAxis, GamepadButton, GamepadId, InputBackend, KeyCode, MouseButton,
};

/// Trait marker for action types. Any `Copy + Eq + Hash + Send + Sync +
/// 'static` enum qualifies — blanket impl below.
pub trait Action: Copy + Eq + Hash + Send + Sync + 'static {}
impl<T: Copy + Eq + Hash + Send + Sync + 'static> Action for T {}

/// One way to trigger an action. An action can have many bindings
/// (e.g. `Jump` ← `Space` and `Jump` ← `GamepadButton::South`).
#[derive(Debug, Clone, Copy)]
pub enum InputBinding {
    Key(KeyCode),
    Mouse(MouseButton),
    GamepadButton(GamepadId, GamepadButton),
    /// Gamepad axis as a boolean trigger above `threshold` magnitude, or
    /// as a continuous value for [`ActionMap::axis_value`]. Sign of
    /// `threshold` determines direction (`> 0` for positive, `< 0` for
    /// negative). For `axis_value`, the raw axis value passes through
    /// (sign + magnitude).
    GamepadAxis {
        gamepad: GamepadId,
        axis: GamepadAxis,
        threshold: f32,
    },
}

/// Bindings registry, keyed by action.
///
/// `bindings[action]` is a `Vec<InputBinding>` — pushing the same action
/// multiple times accumulates bindings rather than overwriting.
pub struct ActionMap<A: Action> {
    bindings: HashMap<A, Vec<InputBinding>>,
}

impl<A: Action> ActionMap<A> {
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
        }
    }

    /// Adds a binding for `action`. Multiple calls accumulate.
    pub fn bind(&mut self, action: A, binding: InputBinding) {
        self.bindings.entry(action).or_default().push(binding);
    }

    /// Drops every binding for `action`.
    pub fn unbind(&mut self, action: A) {
        self.bindings.remove(&action);
    }

    /// Drops every binding everywhere — equivalent to a fresh map.
    pub fn clear(&mut self) {
        self.bindings.clear();
    }

    /// Returns the bindings for `action`, or `&[]` if none.
    pub fn bindings_for(&self, action: A) -> &[InputBinding] {
        self.bindings
            .get(&action)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// `true` when ANY binding for `action` is currently active.
    pub fn is_pressed(&self, action: A, backend: &dyn InputBackend) -> bool {
        let Some(bindings) = self.bindings.get(&action) else {
            return false;
        };
        bindings
            .iter()
            .any(|binding| binding_is_pressed(*binding, backend))
    }

    /// `true` when ANY binding for `action` transitioned to pressed
    /// during the most recent frame.
    pub fn just_pressed(&self, action: A, backend: &dyn InputBackend) -> bool {
        let Some(bindings) = self.bindings.get(&action) else {
            return false;
        };
        bindings
            .iter()
            .any(|binding| binding_just_pressed(*binding, backend))
    }

    /// Strongest signal among bindings, signed. Useful for analog sticks
    /// + WASD-style movement combined into a single `MoveForward` action.
    pub fn axis_value(&self, action: A, backend: &dyn InputBackend) -> f32 {
        let Some(bindings) = self.bindings.get(&action) else {
            return 0.0;
        };
        let mut max_abs = 0.0;
        let mut signed = 0.0;
        for binding in bindings {
            let value = binding_axis_value(*binding, backend);
            if value.abs() > max_abs {
                max_abs = value.abs();
                signed = value;
            }
        }
        signed
    }
}

impl<A: Action> Default for ActionMap<A> {
    fn default() -> Self {
        Self::new()
    }
}

fn binding_is_pressed(binding: InputBinding, backend: &dyn InputBackend) -> bool {
    match binding {
        InputBinding::Key(key) => backend.is_pressed(key),
        InputBinding::Mouse(button) => backend.is_mouse_pressed(button),
        InputBinding::GamepadButton(id, button) => backend.is_button_pressed(id, button),
        InputBinding::GamepadAxis {
            gamepad,
            axis,
            threshold,
        } => {
            let value = backend.axis_value(gamepad, axis);
            if threshold >= 0.0 {
                value >= threshold
            } else {
                value <= threshold
            }
        }
    }
}

fn binding_just_pressed(binding: InputBinding, backend: &dyn InputBackend) -> bool {
    match binding {
        InputBinding::Key(key) => backend.just_pressed(key),
        // Mouse + gamepad just_pressed not yet exposed — backends keep
        // current/previous state for keys only in PR-1. Follow-up will
        // generalize edge detection across all input kinds.
        _ => false,
    }
}

fn binding_axis_value(binding: InputBinding, backend: &dyn InputBackend) -> f32 {
    match binding {
        InputBinding::Key(key) => {
            if backend.is_pressed(key) {
                1.0
            } else {
                0.0
            }
        }
        InputBinding::Mouse(button) => {
            if backend.is_mouse_pressed(button) {
                1.0
            } else {
                0.0
            }
        }
        InputBinding::GamepadButton(id, button) => {
            if backend.is_button_pressed(id, button) {
                1.0
            } else {
                0.0
            }
        }
        InputBinding::GamepadAxis { gamepad, axis, .. } => backend.axis_value(gamepad, axis),
    }
}
