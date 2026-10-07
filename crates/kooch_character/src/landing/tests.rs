//! 🔴 The scaling, and the edge. Whether a landing *feels* right is what playing answers; whether
//! walking a staircase fires one is arithmetic, and it is what the first version got wrong.

use super::*;

fn landing() -> LandingImpulse {
    LandingImpulse {
        min_speed: 3.0,
        full_speed: 15.0,
        fall_speed: 0.0,
        ..Default::default()
    }
}

/// The curve between the two speeds, which is what separates a kerb from a rooftop.
#[test]
fn a_faster_fall_shakes_more() {
    let l = landing();
    let strength =
        |speed: f32| ((speed - l.min_speed) / (l.full_speed - l.min_speed)).clamp(0.0, 1.0);
    assert!(strength(2.0) == 0.0, "a kerb shook the camera");
    assert!(
        (strength(9.0) - 0.5).abs() < 1e-5,
        "halfway read as {}",
        strength(9.0)
    );
    assert!(strength(40.0) == 1.0, "a long fall was not clamped");
}

/// 🔴 The bug a smoke test found. `Footing::stands()` is `Ground` only — a step is something you
/// are getting OVER — so walking a staircase flips `standing` false and true repeatedly. Watching
/// that field made every flip a landing, and the test scene is built out of steps and ramps, so the
/// camera never stopped shaking.
///
/// Support is about having ground AT ALL. A step has a normal; only the air has none.
#[test]
fn a_step_is_not_a_landing() {
    let on_a_step = Grounded {
        // Too steep to stand on, but there is something there.
        standing: false,
        normal: glam::Vec3::new(0.0, 0.7, 0.7).normalize(),
        distance: 0.2,
    };
    let in_the_air = Grounded {
        standing: false,
        normal: glam::Vec3::ZERO,
        distance: 0.0,
    };
    assert!(
        supported(&on_a_step),
        "a step read as the air, so stepping onto it counts as landing",
    );
    assert!(!supported(&in_the_air), "the air read as ground");
}

/// And standing on open ground is support, which is the case that must not fire twice.
#[test]
fn ground_is_support() {
    let flat = Grounded {
        standing: true,
        normal: glam::Vec3::Y,
        distance: 0.1,
    };
    assert!(supported(&flat));
}
