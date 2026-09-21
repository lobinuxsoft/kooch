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
