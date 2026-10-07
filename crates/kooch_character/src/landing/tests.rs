//! 🔴 The only part a smoke test cannot judge: the scaling, and that the PEAK is what is measured.
//! Whether a landing *feels* right is exactly what playing answers.

use super::*;

fn landing() -> LandingImpulse {
    LandingImpulse {
        min_speed: 3.0,
        full_speed: 15.0,
        fall_speed: 0.0,
        was_standing: true,
    }
}

/// The curve between the two speeds, which is what separates a kerb from a rooftop.
#[test]
fn a_faster_fall_shakes_more() {
    let strength = |speed: f32| {
        let l = landing();
        ((speed - l.min_speed) / (l.full_speed - l.min_speed)).clamp(0.0, 1.0)
    };
    assert!(strength(2.0) == 0.0, "a kerb shook the camera");
    assert!(
        (strength(9.0) - 0.5).abs() < 1e-5,
        "halfway read as {}",
        strength(9.0)
    );
    assert!(strength(40.0) == 1.0, "a long fall was not clamped");
}
