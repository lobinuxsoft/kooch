//! Only the scaling, which is the part nobody can eyeball: whether a hit twice as hard shakes
//! twice as much.

use super::*;

fn collision() -> CollisionImpulse {
    CollisionImpulse {
        min_force: 100.0,
        full_force: 1_100.0,
    }
}

fn source() -> ImpulseSource {
    ImpulseSource {
        amplitude: glam::Vec3::new(0.0, 1.0, 0.0),
        ..Default::default()
    }
}

#[test]
fn a_harder_hit_shakes_more() {
    let soft = scaled(source(), 350.0, collision()).amplitude.y;
    let hard = scaled(source(), 850.0, collision()).amplitude.y;
    assert!(
        (soft - 0.25).abs() < 1e-5,
        "a quarter of the way up read as {soft}"
    );
    assert!((hard - 0.75).abs() < 1e-5, "three quarters read as {hard}");
}

/// 🔴 Clamped, or a fall off the map throws the camera into the next county.
#[test]
fn past_full_force_is_clamped() {
    let huge = scaled(source(), 50_000.0, collision()).amplitude.y;
    assert!((huge - 1.0).abs() < 1e-5, "a 50 kN hit scaled to {huge}");
}

/// A zero span is "anything over the floor is a full hit" — a setting, not a division to guard.
#[test]
fn a_zero_span_is_always_full() {
    let flat = CollisionImpulse {
        min_force: 100.0,
        full_force: 100.0,
    };
    assert!((scaled(source(), 200.0, flat).amplitude.y - 1.0).abs() < 1e-5);
}
