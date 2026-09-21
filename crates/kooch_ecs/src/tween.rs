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

/// A value a [`Chase`] moves: blended by progress, and compared to tell a goal that moved.
pub trait Tweened: Copy {
    fn mix(from: Self, to: Self, t: f32) -> Self;
    /// Whether `a` and `b` are different goals, past float noise.
    fn moved(a: Self, b: Self) -> bool;
}

/// Below this a goal has not moved: a settled rig's float noise must not restart its tween.
const STILL: f32 = 1e-5;

impl Tweened for f32 {
    fn mix(from: Self, to: Self, t: f32) -> Self {
        from + (to - from) * t
    }
    fn moved(a: Self, b: Self) -> bool {
        (a - b).abs() > STILL
    }
}

impl Tweened for glam::Vec3 {
    fn mix(from: Self, to: Self, t: f32) -> Self {
        from.lerp(to, t)
    }
    fn moved(a: Self, b: Self) -> bool {
        !a.abs_diff_eq(b, STILL)
    }
}

impl Tweened for glam::Quat {
    /// Along the shorter arc: `q` and `-q` are one rotation, and the long way is 359° of roll.
    fn mix(from: Self, to: Self, t: f32) -> Self {
        let to = if from.dot(to) < 0.0 { -to } else { to };
        from.slerp(to, t).normalize()
    }
    /// By components, not the dot product: near 1 an `f32` dot cannot tell a small turn from none.
    fn moved(a: Self, b: Self) -> bool {
        !a.abs_diff_eq(b, STILL) && !a.abs_diff_eq(-b, STILL)
    }
}

/// A tween towards a goal that may move. A moved goal restarts it from where the value is, so the
/// value arrives exactly `duration` seconds after the goal stops. Sine ease-out, fixed: an ease-in
/// barely leaves while the goal keeps moving, and the value would never keep up.
#[derive(Debug, Clone, Copy)]
pub struct Chase<T> {
    from: T,
    goal: T,
    elapsed: f32,
}

impl<T: Tweened> Chase<T> {
    /// At rest on `at`.
    pub fn at(at: T) -> Self {
        Self {
            from: at,
            goal: at,
            elapsed: 0.0,
        }
    }

    /// Advances `dt` towards `goal` from `current`, answering the new value. Zero `duration` snaps.
    pub fn step(&mut self, current: T, goal: T, dt: f32, duration: f32) -> T {
        if duration <= 0.0 {
            *self = Self::at(goal);
            return goal;
        }
        if T::moved(goal, self.goal) {
            *self = Self {
                from: current,
                goal,
                elapsed: 0.0,
            };
        }
        self.elapsed += dt;
        // A hair of slack so a clock summed in `f32` steps lands on 1 instead of just short of it.
        let t = eased(self.elapsed / duration + 1e-5, CURVE_SINE, EASE_OUT);
        T::mix(self.from, self.goal, t)
    }
}

#[cfg(test)]
mod tests;
