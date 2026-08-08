//! [`Processor`] — what happens to a value between the device and the
//! action.
//!
//! # One place, not three
//!
//! Unity lets processors sit on the control's layout, on the action, and
//! on the binding, and applies all three in sequence. A stick therefore
//! arrives with the layout's deadzone already applied, and a binding that
//! adds its own gets two — which is a known source of "my stick feels
//! wrong" and something their own source questions in a `////REVIEW` on
//! line 7 of `InputBinding.cs`.
//!
//! Here a processor lives on the binding and nowhere else. The device
//! hands over a raw value; everything that shapes it is visible in one
//! list, in order.
//!
//! # Typed, not a string
//!
//! Unity stores `"axisDeadzone(min=0.1,max=0.95);invert"` as text. That
//! is convenient for an editor and unkind to everyone else: a misspelt
//! processor is not an error, it is a processor that silently does not
//! exist. The dropdown in the panel offers the same list either way.
//!
//! The formulas below are ported from
//! `com.unity.inputsystem@1.20/InputSystem/Runtime/Controls/Processors/`,
//! because "what feels right on a stick" is tuning that took a decade and
//! is not worth re-deriving.

use glam::Vec2;
use serde::{Deserialize, Serialize};

/// The default a deadzone uses when its bounds are left unset.
///
/// Unity's global defaults. `min` cuts the slop a stick reports at rest;
/// `max` is where it should already count as fully pushed, since a worn
/// stick rarely reaches 1.0 in the corners.
pub const DEFAULT_DEADZONE_MIN: f32 = 0.125;
pub const DEFAULT_DEADZONE_MAX: f32 = 0.925;

/// One step of shaping between a control and an action's value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Processor {
    /// Deadzone on a single axis, applied per component.
    ///
    /// ⚠️ On a stick this is the wrong one and the mistake is invisible
    /// until you draw it: cutting each axis independently leaves a
    /// **square** hole, so a stick pushed diagonally registers while the
    /// same push along an axis does not. Use [`Processor::StickDeadzone`]
    /// for anything two-dimensional (#57).
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
    /// Caps a vector's length at 1 **without stretching shorter ones**.
    ///
    /// The one a keyboard needs: pressing two directions must not travel
    /// 1.41× faster than one, and a half-held stick must stay half. Unity
    /// gets the same effect inside its 2D composite rather than as a
    /// processor, by multiplying diagonals by `0.707107` — which is exact
    /// only because its inputs are 0 or ±1 there. As a processor it has
    /// to handle any length, so it is the honest form.
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

    /// Whether this does anything to a value of `control_type`.
    ///
    /// The 2D processors are skipped by [`apply`](Self::apply), so on a
    /// button or an axis they are a row that shapes nothing. Unity
    /// filters its own menu by the expected value type for the same
    /// reason: offering one is offering a setting that reads as broken.
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

    /// The same, in three dimensions.
    ///
    /// The 2D processors act on `xy` and leave `z` alone rather than
    /// refusing to run: a stick deadzone applied to a 3D composite is
    /// about the stick, and zeroing the third axis because the processor
    /// predates it would make the binding read as broken.
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

/// Ported from Unity's `AxisDeadzoneProcessor`.
///
/// Below `min` is nothing; above `max` is already full; between them the
/// range is stretched so the value leaves the deadzone at 0 rather than
/// jumping to `min`. That last part is what stops the visible step when
/// a stick crosses the threshold.
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

/// Ported from Unity's `StickDeadzoneProcessor`.
///
/// The same curve applied to the vector's **length**, with the direction
/// preserved — which is what makes the hole round instead of square.
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
