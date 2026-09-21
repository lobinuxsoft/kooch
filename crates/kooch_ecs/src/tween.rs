//! Tweening: the five curves that get used and which end of them is slow, ported from
//! phantom-camera, for anything that moves from one value to another over a time.
//!
//! A component tweens with two `u32` fields — `#[reflect(choices = CURVE_CHOICES)]` and
//! `#[reflect(choices = EASE_CHOICES)]` — and [`eased`] maps its progress. No overshoot curves:
//! overshoot on a gameplay camera reads as a mistake, and it was a camera that needed them first.

use crate::reflect::FieldChoice;

/// Constant speed.
pub const CURVE_LINEAR: u32 = 0;
/// Gentle, and the one that reads as "smooth" without being asked.
pub const CURVE_SINE: u32 = 1;
/// `t²` — a mild acceleration.
pub const CURVE_QUAD: u32 = 2;
/// `t³` — a stronger one.
pub const CURVE_CUBIC: u32 = 3;
/// Nearly a cut that softens at one end.
pub const CURVE_EXPO: u32 = 4;

/// Starts slow, arrives fast.
pub const EASE_IN: u32 = 0;
/// Starts fast, arrives slow.
pub const EASE_OUT: u32 = 1;
/// Slow at both ends.
pub const EASE_IN_OUT: u32 = 2;

/// Labels for a curve dropdown.
pub static CURVE_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "Linear",
        value: CURVE_LINEAR as i64,
    },
    FieldChoice {
        label: "Sine",
        value: CURVE_SINE as i64,
    },
    FieldChoice {
        label: "Quadratic",
        value: CURVE_QUAD as i64,
    },
    FieldChoice {
        label: "Cubic",
        value: CURVE_CUBIC as i64,
    },
    FieldChoice {
        label: "Exponential",
        value: CURVE_EXPO as i64,
    },
];

/// Labels for an ease dropdown.
pub static EASE_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "Ease in",
        value: EASE_IN as i64,
    },
    FieldChoice {
        label: "Ease out",
        value: EASE_OUT as i64,
    },
    FieldChoice {
        label: "Ease in-out",
        value: EASE_IN_OUT as i64,
    },
];

/// Maps linear progress to eased progress; `t` is clamped so an overshoot ends on the curve. Each
/// curve is written as ease-in and the others mirror it, so `out(t) = 1 - in(1 - t)` holds exactly.
pub fn eased(t: f32, curve: u32, ease: u32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match ease {
        EASE_OUT => 1.0 - ease_in(1.0 - t, curve),
        EASE_IN_OUT => {
            if t < 0.5 {
                ease_in(t * 2.0, curve) * 0.5
            } else {
                1.0 - ease_in((1.0 - t) * 2.0, curve) * 0.5
            }
        }
        _ => ease_in(t, curve),
    }
}

/// The ease-in form of each curve, on `[0, 1]`.
fn ease_in(t: f32, curve: u32) -> f32 {
    match curve {
        CURVE_SINE => 1.0 - (t * std::f32::consts::FRAC_PI_2).cos(),
        CURVE_QUAD => t * t,
        CURVE_CUBIC => t * t * t,
        // Anchored so that `f(0) = 0` exactly; `2^(10(t-1))` alone leaves
        // a visible step of 1/1024 at the start.
        CURVE_EXPO => {
            if t <= 0.0 {
                0.0
            } else {
                (2.0_f32).powf(10.0 * (t - 1.0)) - 0.001
            }
        }
        _ => t,
    }
}

#[cfg(test)]
mod tests;
