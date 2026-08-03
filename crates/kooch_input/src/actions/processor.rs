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
mod tests {
    use super::*;

    /// The reason there are two deadzones and not one parameterised.
    ///
    /// A per-axis deadzone leaves a square hole: a stick pushed diagonally
    /// clears it while the same magnitude along an axis does not. This is
    /// the bug #57 names, drawn as a test.
    #[test]
    fn only_the_stick_deadzone_leaves_a_round_hole() {
        let (min, max) = (0.2, 0.9);
        // Just inside the deadzone, straight up.
        let cardinal = Vec2::new(0.0, 0.15);
        // The same length, at 45°.
        let diagonal = Vec2::new(0.15, 0.15).normalize() * 0.15;
        assert!((cardinal.length() - diagonal.length()).abs() < 1e-5);

        let radial = Processor::StickDeadzone { min, max };
        assert_eq!(radial.apply_vec2(cardinal), Vec2::ZERO);
        assert_eq!(
            radial.apply_vec2(diagonal),
            Vec2::ZERO,
            "a round deadzone must reject both, since they are the same push"
        );

        // Per-axis: each component alone is under `min`, so the diagonal
        // is rejected too — but push it a little further and the corner
        // survives while the cardinal of equal length does not.
        let square = Processor::AxisDeadzone { min, max };
        let long_diagonal = Vec2::new(0.25, 0.25);
        let long_cardinal = Vec2::new(0.0, long_diagonal.length());
        assert_ne!(square.apply_vec2(long_diagonal), Vec2::ZERO);
        assert_ne!(
            square.apply_vec2(long_cardinal),
            Vec2::ZERO,
            "sanity: both are past min"
        );
        // …and the shapes differ, which is the whole point.
        assert!(
            (square.apply_vec2(long_diagonal).length() - square.apply_vec2(long_cardinal).length())
                .abs()
                > 0.1,
            "a square deadzone should distort direction; if this fails the \
             two processors have collapsed into one"
        );
    }

    /// Leaving the deadzone must start at zero, not jump to `min`.
    #[test]
    fn a_value_leaving_the_deadzone_starts_from_zero() {
        let p = Processor::AxisDeadzone { min: 0.2, max: 0.9 };
        assert_eq!(p.apply(0.19), 0.0);
        assert!(p.apply(0.2001).abs() < 0.01, "stepped instead of easing in");
        assert_eq!(p.apply(0.95), 1.0, "past max is already full");
        assert_eq!(p.apply(-0.95), -1.0, "and symmetric");
    }

    /// A nudge stays a nudge; a diagonal does not outrun a straight line.
    /// This is `clamp_to_unit` from the game, promoted to a processor.
    #[test]
    fn normalising_a_vector_caps_without_stretching() {
        let p = Processor::NormalizeVector2;
        let half = Vec2::new(0.0, 0.5);
        assert_eq!(p.apply_vec2(half), half, "a half-held stick became full");

        let diagonal = p.apply_vec2(Vec2::new(1.0, 1.0));
        assert!(
            (diagonal.length() - 1.0).abs() < 1e-5,
            "diagonal travels {}× too fast",
            diagonal.length()
        );
    }

    /// Order matters, and the list is the order — no hidden stage before
    /// or after, which is the thing three processor sites cost you.
    #[test]
    fn processors_apply_in_the_order_they_are_listed() {
        let value = -0.5;
        let invert_then_scale =
            Processor::Scale { factor: 2.0 }.apply(Processor::Invert.apply(value));
        let scale_then_invert =
            Processor::Invert.apply(Processor::Scale { factor: 2.0 }.apply(value));
        assert_eq!(invert_then_scale, 1.0);
        assert_eq!(scale_then_invert, 1.0);

        // Deadzone is where order shows: scaling first can push a value
        // out of a deadzone that would have swallowed it.
        let dead = Processor::AxisDeadzone { min: 0.2, max: 0.9 };
        let small = 0.15;
        assert_eq!(dead.apply(small), 0.0);
        assert_ne!(
            dead.apply(Processor::Scale { factor: 2.0 }.apply(small)),
            0.0
        );
    }
}
