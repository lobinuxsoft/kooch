//! 🔴 The shapes are Cinemachine's keyframes and the evaluation is its Hermite. Those numbers are
//! what makes a bump feel like a bump, and a signal read with the wrong basis looks right on a
//! graph while feeling wrong to play — which a smoke test reports as "it's off" and nothing more.

use super::*;

/// Every shape starts and ends where Cinemachine's does, or the signal does not close.
#[test]
fn every_shape_starts_and_ends_right() {
    for (shape, start) in [(RECOIL, 1.0), (BUMP, 0.0), (EXPLOSION, -1.4), (RUMBLE, 0.0)] {
        assert!(
            (at(shape, 0.0) - start).abs() < 1e-5,
            "shape {shape} starts at {}",
            at(shape, 0.0)
        );
        assert_eq!(at(shape, 1.0), 0.0, "shape {shape} does not end at rest");
        // Past its window it contributes nothing, which is what lets a finished impulse be dropped.
        assert_eq!(
            at(shape, 1.5),
            0.0,
            "shape {shape} rings on past its duration"
        );
    }
}

/// 🔴 The tangents must be scaled by the span, and this asserts the VALUE rather than the sign —
/// the first version only asked for "negative", and a reading that forgot the scale passed it while
/// being five times too strong.
///
/// Worked by hand from `BUMP_KEYS`. At `t = 0.1` the span is 0.2 and `u` is 0.5, so the Hermite
/// basis gives `h10 = 0.125` and `h11 = -0.125`, and with both keyframe values at zero the whole
/// result is `0.125 · 0.2 · (-4.9) + (-0.125) · 0.2 · 8.25 = -0.329`. Dropping the `0.2` gives
/// `-1.64`: the same shape, five times too big, and nothing about the sign to notice it by.
#[test]
fn bump_is_its_tangents_to_scale() {
    let dip = at(BUMP, 0.1);
    assert!(
        (dip - -0.32875).abs() < 1e-4,
        "the dip is {dip}, not -0.329 — the span scale is wrong",
    );
    assert!(
        at(BUMP, 0.5) > 0.05,
        "it never rises, so the tangents are ignored"
    );
}

/// 🔴 Explosion starts on the back foot and its first swing overshoots forward. The values are
/// Cinemachine's, so this is the shape being read with the right basis rather than a plausible one.
#[test]
fn explosion_swings_through_its_keyframes() {
    assert!(
        (at(EXPLOSION, 0.27) - 0.78).abs() < 1e-4,
        "its peak is {}",
        at(EXPLOSION, 0.27)
    );
    assert!(
        (at(EXPLOSION, 0.54) - -0.12).abs() < 1e-4,
        "its rebound is {}",
        at(EXPLOSION, 0.54)
    );
}

/// Rumble's peaks are at its keyframes, with zero tangents — the one shape a linear reading would
/// get nearly right, and the one that proves the keyframe lookup picks the right span.
#[test]
fn rumble_peaks_where_it_should() {
    assert!(
        (at(RUMBLE, 0.5) - 1.0).abs() < 1e-4,
        "its tallest swell is {}",
        at(RUMBLE, 0.5)
    );
    assert!(
        at(RUMBLE, 0.2).abs() < 1e-4,
        "it does not return to rest between swells"
    );
}

/// The same input gives the same output, which is the whole reason this is a curve and not noise.
#[test]
fn the_signal_is_deterministic() {
    for shape in [RECOIL, BUMP, EXPLOSION, RUMBLE] {
        for step in 0..20 {
            let t = step as f32 / 20.0;
            assert_eq!(at(shape, t), at(shape, t));
        }
    }
}
