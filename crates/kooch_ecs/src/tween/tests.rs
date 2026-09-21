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

/// Steps a chase to a goal that stays put, `steps` times at `dt`.
fn chased(duration: f32, dt: f32, steps: usize) -> f32 {
    let mut chase = Chase::at(0.0_f32);
    let mut value = 0.0;
    for _ in 0..steps {
        value = chase.step(value, 10.0, dt, duration);
    }
    value
}

/// The duration is when it arrives — exactly, at 30 fps or 144 — and not a moment before.
#[test]
fn a_chase_arrives_on_time() {
    for fps in [30.0_f32, 60.0, 144.0] {
        let steps = (0.5 * fps).round() as usize;
        assert_eq!(chased(0.5, 1.0 / fps, steps), 10.0, "{fps} fps");
        assert!(
            chased(0.5, 1.0 / fps, steps - 1) < 10.0,
            "{fps} fps arrived early"
        );
    }
}

/// A goal that moves restarts the clock from where the value is: arriving is measured from when it
/// stopped.
#[test]
fn a_moved_goal_restarts() {
    let dt = 1.0 / 60.0;
    let mut chase = Chase::at(0.0_f32);
    let mut value = 0.0;
    for _ in 0..20 {
        value = chase.step(value, 10.0, dt, 0.5);
    }
    let midway = value;
    for step in 0..30 {
        value = chase.step(value, 20.0, dt, 0.5);
        if step < 29 {
            assert!(value < 20.0, "arrived at step {step}");
        }
    }
    assert!(midway > 0.0 && midway < 10.0);
    assert_eq!(value, 20.0);
}

#[test]
fn a_zero_duration_snaps() {
    let mut chase = Chase::at(0.0_f32);
    assert_eq!(chase.step(0.0, 3.0, 1.0 / 60.0, 0.0), 3.0);
}

/// A goal creeping less than the noise floor each step is still followed: moves are measured from
/// the goal the tween holds, so they add up until they count.
#[test]
fn a_creeping_goal_is_followed() {
    let mut chase = Chase::at(0.0_f32);
    let (mut value, mut goal) = (0.0, 0.0);
    for _ in 0..2000 {
        goal += 5e-6;
        value = chase.step(value, goal, 1.0 / 60.0, 0.2);
    }
    // Trailing a moving goal by at most its speed times the duration: 3e-4 m/s over 0.2 s.
    assert!((goal - value).abs() < 6e-5, "left behind: {value} vs {goal}");
}
