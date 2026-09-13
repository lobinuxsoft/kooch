//! Reading an [`ActionMap`] against a backend, once per frame. The most actuated binding wins, as
//! in Unity — the same rule lets a higher map consume an action.
//! State, never events, so a dropped frame self-corrects (#711, #713).

use glam::{Vec2, Vec3};

use super::action::{Action, ActionId, ActionMap, ControlType};
use super::binding::{Binding, BothHeld, Composite, Group, PartName, VectorMode, groups};
use super::path::ControlPath;
use crate::backend::InputBackend;
use crate::ids::GamepadId;

/// What one action is worth this frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ActionValue {
    /// The full value. A button uses `x` as 0 or 1, an axis uses `x`,
    /// a 2D composite `xy`. Three components so a 3D composite has
    /// somewhere to land — see [`ControlType::Vector3`].
    pub vector: Vec3,
    /// Whether it counts as held. For an axis, past halfway.
    pub pressed: bool,
}

impl ActionValue {
    pub fn axis(self) -> f32 {
        self.vector.x
    }

    /// The first two components. What a 2D action wants, and what a 3D
    /// one gives up when read as flat.
    pub fn vector2(self) -> Vec2 {
        self.vector.truncate()
    }
}

/// Reads one action against a backend, with no map involved — evaluation never needed one.
pub fn evaluate(action: &Action, backend: &dyn InputBackend) -> ActionValue {
    let pad = backend.gamepads().first().copied();
    read_action(action, backend, pad)
}

/// Reads every binding group and keeps the most actuated.
fn read_action(action: &Action, backend: &dyn InputBackend, pad: Option<GamepadId>) -> ActionValue {
    let control_type = action.control_type;
    let mut best = Vec3::ZERO;
    let mut best_magnitude = 0.0;

    for group in groups(&action.bindings) {
        let raw = match group {
            Group::Single { path, binding } => {
                let value = read_control(path, backend, pad);
                apply(binding, Vec3::new(value, 0.0, 0.0))
            }
            Group::Composite {
                composite,
                head,
                parts,
            } => apply(head, read_composite(composite, parts, backend, pad)),
        };
        let magnitude = raw.length();
        if magnitude > best_magnitude {
            best_magnitude = magnitude;
            best = raw;
        }
    }

    // The action's processors, once on the winning value rather than per binding, so nothing is
    // applied twice.
    let value = action
        .processors
        .iter()
        .fold(best, |acc, processor| processor.apply_vec3(acc));

    ActionValue {
        vector: value,
        // Measured after those processors, not before: an action scaled
        // to zero is not held, and one clamped up is.
        pressed: match control_type {
            ControlType::Vector2 | ControlType::Vector3 => value.length() > 0.5,
            _ => value.x.abs() > 0.5,
        },
    }
}

/// Runs a binding's processors, in order.
fn apply(binding: &Binding, value: Vec3) -> Vec3 {
    binding
        .processors
        .iter()
        .fold(value, |acc, processor| processor.apply_vec3(acc))
}

fn read_composite(
    composite: Composite,
    parts: &[Binding],
    backend: &dyn InputBackend,
    pad: Option<GamepadId>,
) -> Vec3 {
    let part = |name: PartName| -> f32 {
        parts
            .iter()
            .find(|binding| matches!(binding.role, super::binding::Role::Part { name: n, .. } if n == name))
            .and_then(|binding| binding.path().map(|path| (binding, path)))
            .map(|(binding, path)| {
                apply(binding, Vec3::new(read_control(path, backend, pad), 0.0, 0.0)).x
            })
            .unwrap_or(0.0)
    };

    match composite {
        Composite::Axis1D { both_held } => {
            let positive = part(PartName::Positive);
            let negative = part(PartName::Negative);
            let both = positive.abs() > 0.5 && negative.abs() > 0.5;
            let value = match (both, both_held) {
                (true, BothHeld::Neither) => 0.0,
                (true, BothHeld::Positive) => positive,
                (true, BothHeld::Negative) => -negative,
                (false, _) => positive - negative,
            };
            Vec3::new(value, 0.0, 0.0)
        }
        Composite::Vector2 { mode } => {
            let (up, down) = (part(PartName::Up), part(PartName::Down));
            let (left, right) = (part(PartName::Left), part(PartName::Right));
            normalized(Vec3::new(right - left, up - down, 0.0), mode)
        }
        Composite::Vector3 { mode } => {
            let (up, down) = (part(PartName::Up), part(PartName::Down));
            let (left, right) = (part(PartName::Left), part(PartName::Right));
            let (forward, back) = (part(PartName::Forward), part(PartName::Backward));
            normalized(Vec3::new(right - left, up - down, forward - back), mode)
        }
        // The gate reads as a button even when bound to an axis, matching
        // Unity: a trigger half-pulled is not a held modifier.
        Composite::OneModifier => gated(part(PartName::Modifier) > 0.5, part(PartName::Value)),
        Composite::TwoModifiers => gated(
            part(PartName::Modifier) > 0.5 && part(PartName::Modifier2) > 0.5,
            part(PartName::Value),
        ),
    }
}

/// Caps a composite's raw sum at length 1 when its parts are buttons, or diagonals run 1.41× faster
/// (1.73× in 3D).
fn normalized(raw: Vec3, mode: VectorMode) -> Vec3 {
    match mode {
        // A stick already reports how far it is pushed; normalising
        // would throw that away.
        VectorMode::Analog | VectorMode::Digital => raw,
        VectorMode::DigitalNormalized => {
            if raw.length_squared() > 1.0 {
                raw.normalize()
            } else {
                raw
            }
        }
    }
}

/// A modifier composite's value: the gated part, or nothing.
fn gated(open: bool, value: f32) -> Vec3 {
    if open {
        Vec3::new(value, 0.0, 0.0)
    } else {
        Vec3::ZERO
    }
}

/// One control's value as a number: a button reads 0 or 1, so a d-pad and a stick are
/// interchangeable in a binding.
fn read_control(path: ControlPath, backend: &dyn InputBackend, pad: Option<GamepadId>) -> f32 {
    match path {
        ControlPath::Key(key) => backend.is_pressed(key) as u8 as f32,
        ControlPath::Mouse(button) => backend.is_mouse_pressed(button) as u8 as f32,
        ControlPath::Button(button) => pad
            .map(|pad| backend.is_button_pressed(pad, button) as u8 as f32)
            .unwrap_or(0.0),
        ControlPath::Axis(axis) => pad.map(|pad| backend.axis_value(pad, axis)).unwrap_or(0.0),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod composite_tests;
