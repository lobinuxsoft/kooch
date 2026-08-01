//! Easing for the transition between two virtual cameras.
//!
//! Ported from the curve set phantom-camera exposes, which is Godot's
//! `Tween` vocabulary — familiar to anyone who has authored a transition
//! before. Godot hands its addon eleven curve types for free because
//! `Tween` implements them; here each one is code, so the set stops at
//! the five that get used. `Elastic`, `Bounce` and `Back` are missing on
//! purpose: overshoot on a gameplay camera reads as a mistake, and
//! adding one later changes no shape.

use kooch_ecs::reflect::FieldChoice;

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

/// Labels for the `blend_curve` dropdown.
pub static BLEND_CURVE_CHOICES: &[FieldChoice] = &[
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

/// Labels for the `blend_ease` dropdown.
pub static BLEND_EASE_CHOICES: &[FieldChoice] = &[
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

/// Maps linear progress to eased progress.
///
/// `t` is clamped, so a caller that overshoots its duration gets the end
/// of the curve rather than an extrapolation past the destination.
///
/// Every curve is expressed as its ease-in form and the other two are
/// derived by mirroring, which is how the identities `out(t) = 1 -
/// in(1-t)` hold exactly. Writing three variants per curve by hand is
/// how one of them ends up subtly different from its siblings.
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
mod tests {
    use super::*;

    const CURVES: [u32; 5] = [
        CURVE_LINEAR,
        CURVE_SINE,
        CURVE_QUAD,
        CURVE_CUBIC,
        CURVE_EXPO,
    ];
    const EASES: [u32; 3] = [EASE_IN, EASE_OUT, EASE_IN_OUT];

    /// A blend that does not start where it started, or end where it was
    /// going, is a visible jump at one end or the other.
    #[test]
    fn every_curve_runs_from_zero_to_one() {
        for c in CURVES {
            for e in EASES {
                assert!(
                    eased(0.0, c, e).abs() < 1e-3,
                    "curve {c} ease {e} starts at {}",
                    eased(0.0, c, e),
                );
                assert!(
                    (eased(1.0, c, e) - 1.0).abs() < 1e-3,
                    "curve {c} ease {e} ends at {}",
                    eased(1.0, c, e),
                );
            }
        }
    }

    /// A camera that goes backwards mid-transition looks broken. None of
    /// these curves overshoot, which is why the overshooting ones were
    /// left out.
    #[test]
    fn no_curve_moves_backwards_or_overshoots() {
        for c in CURVES {
            for e in EASES {
                let mut prev = 0.0;
                for step in 0..=100 {
                    let v = eased(step as f32 / 100.0, c, e);
                    assert!(v >= prev - 1e-4, "curve {c} ease {e} went back at {step}");
                    assert!((-1e-3..=1.0 + 1e-3).contains(&v), "curve {c} ease {e}: {v}");
                    prev = v;
                }
            }
        }
    }

    /// Out is in, mirrored. Asserting it is what keeps the two from
    /// drifting apart if a curve is ever edited.
    #[test]
    fn ease_out_mirrors_ease_in() {
        for c in CURVES {
            for step in 0..=10 {
                let t = step as f32 / 10.0;
                assert!(
                    (eased(t, c, EASE_OUT) - (1.0 - eased(1.0 - t, c, EASE_IN))).abs() < 1e-5,
                    "curve {c} is not mirrored at {t}",
                );
            }
        }
    }

    #[test]
    fn in_out_is_symmetric_about_the_midpoint() {
        for c in CURVES {
            assert!(
                (eased(0.5, c, EASE_IN_OUT) - 0.5).abs() < 1e-3,
                "curve {c} is not half-way at half-time",
            );
        }
    }

    /// Progress past the end must clamp, not extrapolate past the
    /// destination — one frame of overshoot is a visible flick.
    #[test]
    fn progress_is_clamped() {
        for c in CURVES {
            assert_eq!(eased(1.5, c, EASE_IN_OUT), eased(1.0, c, EASE_IN_OUT));
            assert_eq!(eased(-0.5, c, EASE_IN_OUT), eased(0.0, c, EASE_IN_OUT));
        }
    }

    #[test]
    fn linear_is_the_identity() {
        for step in 0..=10 {
            let t = step as f32 / 10.0;
            assert!((eased(t, CURVE_LINEAR, EASE_IN) - t).abs() < 1e-6);
        }
    }
}
