//! [`Processor`] — what happens to a value between device and action, on the binding only and as a
//! typed enum, so a misspelt processor cannot silently vanish.
//! Formulas ported from Unity's Input System.

use glam::Vec2;
use serde::{Deserialize, Serialize};

/// Default deadzone bounds, Unity's: `min` cuts resting slop, `max` counts a worn stick as fully
/// pushed.
pub const DEFAULT_DEADZONE_MIN: f32 = 0.125;
pub const DEFAULT_DEADZONE_MAX: f32 = 0.925;

/// One step of shaping between a control and an action's value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Processor {
    /// Deadzone on a single axis, per component. ⚠️ On a stick it leaves a square hole — use
    /// [`Processor::StickDeadzone`] for anything 2D (#57).
    AxisDeadzone { min: f32, max: f32 },
    /// Deadzone on a vector's **magnitude**, which leaves a round hole.
    StickDeadzone { min: f32, max: f32 },
    /// Hard limit, after everything else.
    Clamp { min: f32, max: f32 },
    /// Flips the sign — an inverted Y axis is this and nothing else.
    Invert,
    /// Per-component flip for a vector.
    InvertVector2 { x: bool, y: bool },
    /// Rescales `[min, max]` onto `[0, 1]`, with `zero` mapping to 0.
    Normalize { min: f32, max: f32, zero: f32 },
    /// Caps a vector's length at 1 without stretching shorter ones, so two keys are not 1.41×
    /// faster and a half stick stays half.
    NormalizeVector2,
    /// Multiplies by a constant — sensitivity.
    Scale { factor: f32 },
    /// Per-component scale, for different sensitivity horizontally and
    /// vertically.
    ScaleVector2 { x: f32, y: f32 },
}

impl Processor {
    /// One of each, with sensible defaults — what an editor's "add
    /// processor" menu offers.
    pub const ALL: &'static [Self] = &[
        Self::StickDeadzone {
            min: DEFAULT_DEADZONE_MIN,
            max: DEFAULT_DEADZONE_MAX,
        },
        Self::AxisDeadzone {
            min: DEFAULT_DEADZONE_MIN,
            max: DEFAULT_DEADZONE_MAX,
        },
        Self::Invert,
        Self::InvertVector2 { x: false, y: true },
        Self::NormalizeVector2,
        Self::Normalize {
            min: 0.0,
            max: 1.0,
            zero: 0.0,
        },
        Self::Scale { factor: 1.0 },
        Self::ScaleVector2 { x: 1.0, y: 1.0 },
        Self::Clamp {
            min: -1.0,
            max: 1.0,
        },
    ];

    /// Name for a menu entry.
    pub const fn label(self) -> &'static str {
        match self {
            Self::AxisDeadzone { .. } => "Axis Deadzone",
            Self::StickDeadzone { .. } => "Stick Deadzone",
            Self::Clamp { .. } => "Clamp",
            Self::Invert => "Invert",
            Self::InvertVector2 { .. } => "Invert Vector 2",
            Self::Normalize { .. } => "Normalize",
            Self::NormalizeVector2 => "Normalize Vector 2",
            Self::Scale { .. } => "Scale",
            Self::ScaleVector2 { .. } => "Scale Vector 2",
        }
    }

    /// Whether this does anything to a value of `control_type`; 2D processors on a button shape
    /// nothing, so the menu hides them.
    pub const fn applies_to(self, control_type: super::action::ControlType) -> bool {
        use super::action::ControlType;
        match self {
            Self::StickDeadzone { .. }
            | Self::InvertVector2 { .. }
            | Self::NormalizeVector2
            | Self::ScaleVector2 { .. } => {
                matches!(control_type, ControlType::Vector2 | ControlType::Vector3)
            }
            _ => true,
        }
    }

    /// Applies this step to a scalar. Vector-only processors pass through.
    pub fn apply(self, value: f32) -> f32 {
        match self {
            Processor::AxisDeadzone { min, max } => axis_deadzone(value, min, max),
            Processor::Clamp { min, max } => value.clamp(min, max),
            Processor::Invert => -value,
            Processor::Normalize { min, max, zero } => normalize(value, min, max, zero),
            Processor::Scale { factor } => value * factor,
            // Shaping a two-dimensional value says nothing about one
            // number, so these leave it alone rather than guessing.
            Processor::StickDeadzone { .. }
            | Processor::InvertVector2 { .. }
            | Processor::NormalizeVector2
            | Processor::ScaleVector2 { .. } => value,
        }
    }

    /// Applies this step to a vector. Scalar processors act per component.
    pub fn apply_vec2(self, value: Vec2) -> Vec2 {
        match self {
            Processor::StickDeadzone { min, max } => stick_deadzone(value, min, max),
            Processor::InvertVector2 { x, y } => Vec2::new(
                if x { -value.x } else { value.x },
                if y { -value.y } else { value.y },
            ),
            Processor::NormalizeVector2 => {
                if value.length_squared() > 1.0 {
                    value.normalize()
                } else {
                    value
                }
            }
            Processor::ScaleVector2 { x, y } => Vec2::new(value.x * x, value.y * y),
            other => Vec2::new(other.apply(value.x), other.apply(value.y)),
        }
    }

    /// The same in three dimensions: 2D processors act on `xy` and leave `z` alone rather than
    /// zeroing it.
    pub fn apply_vec3(self, value: glam::Vec3) -> glam::Vec3 {
        match self {
            Processor::StickDeadzone { .. }
            | Processor::InvertVector2 { .. }
            | Processor::ScaleVector2 { .. } => self.apply_vec2(value.truncate()).extend(value.z),
            Processor::NormalizeVector2 => {
                if value.length_squared() > 1.0 {
                    value.normalize()
                } else {
                    value
                }
            }
            other => glam::Vec3::new(
                other.apply(value.x),
                other.apply(value.y),
                other.apply(value.z),
            ),
        }
    }
}

/// Unity's `AxisDeadzoneProcessor`: nothing below `min`, full above `max`, stretched between so the
/// value leaves the deadzone at 0 without a step.
fn axis_deadzone(value: f32, min: f32, max: f32) -> f32 {
    let magnitude = value.abs();
    if magnitude < min {
        return 0.0;
    }
    if magnitude > max {
        return value.signum();
    }
    value.signum() * ((magnitude - min) / (max - min))
}

/// Unity's `StickDeadzoneProcessor`: the same curve on the vector's length, direction kept, so the
/// hole is round.
fn stick_deadzone(value: Vec2, min: f32, max: f32) -> Vec2 {
    let magnitude = value.length();
    if magnitude == 0.0 {
        return Vec2::ZERO;
    }
    let adjusted = axis_deadzone(magnitude, min, max);
    if adjusted == 0.0 {
        return Vec2::ZERO;
    }
    value * (adjusted / magnitude)
}

/// Ported from Unity's `NormalizeProcessor`.
fn normalize(value: f32, min: f32, max: f32, zero: f32) -> f32 {
    if max - min == 0.0 {
        return 0.0;
    }
    if value >= zero {
        let span = max - zero;
        if span == 0.0 {
            return 0.0;
        }
        ((value - zero) / span).clamp(0.0, 1.0)
    } else {
        let span = zero - min;
        if span == 0.0 {
            return 0.0;
        }
        -((zero - value) / span).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests;
