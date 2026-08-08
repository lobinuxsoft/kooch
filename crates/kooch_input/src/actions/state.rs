//! Reading a [`ActionMap`] against a backend, once per frame.
//!
//! # Why the most actuated binding wins
//!
//! An action with several bindings has to decide what happens when more
//! than one is actuated. Two answers are defensible:
//!
//! - **Sum them.** What roll-a-ball did by hand: add the keyboard's
//!   direction to the stick's and cap the result. Holding `W` while
//!   pushing the stick right produces a diagonal nobody asked for.
//! - **The strongest wins.** What Unity does. `W` and a stick pushed
//!   further apart give the stick; `W` alone gives `W`.
//!
//! The second is taken, for a reason beyond taste: the same "who wins"
//! machinery is what lets a map on top **consume** an action so the map
//! below stops seeing it (see [`ActionMap::priority`]). Two mechanisms
//! for one question would drift.
//!
//! # State, never events
//!
//! [`ActionState`] holds what is true *now*, and edges are derived by
//! comparing against the previous frame. A dropped frame therefore
//! self-corrects, where a queue of events would leave an action stuck
//! down forever — the failure that #711 and #713 both were, once at the
//! backend and once across the wire.

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

/// Reads one action against a backend, with no map involved.
///
/// A map is a way to group actions that turn on and off together; it is
/// not a requirement for evaluating one. Unity draws the same line — an
/// action can "stand on its own", and internally it wraps it in a map of
/// one, because *"to the action system, there are no actions without
/// action maps"*. Here there is no wrapper: the evaluator never needed
/// the map, only the action.
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

    // The action's own processors, run once on the value that won rather
    // than on each binding. Unity applies its equivalents per binding,
    // which is how a stick ends up with two deadzones; once at the end
    // there is nothing to double, and a normalize or a sensitivity is
    // written in one place instead of on every binding.
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

/// Caps a composite's raw sum at length 1 when its parts are buttons.
///
/// Without it a diagonal travels 1.41× faster than a straight line —
/// 1.73× in three dimensions, where three keys can be held at once.
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

/// One control's current value, as a number.
///
/// A button reads 0 or 1, so a button bound where an axis is expected
/// behaves like a stick pushed fully — which is what makes a d-pad and a
/// stick interchangeable in a binding.
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
