//! 🔴 Only what the eye cannot judge: the falloff curve and the channel filter. Whether a shake
//! *feels* like a landing is exactly what a smoke test answers and a test cannot.

use super::*;

fn source() -> ImpulseSource {
    ImpulseSource {
        shape: shape::RECOIL,
        amplitude: Vec3::new(0.0, 1.0, 0.0),
        duration: 1.0,
        radius: 5.0,
        dissipation: 10.0,
        channels: 0b1,
    }
}

fn listener() -> ImpulseListener {
    ImpulseListener {
        gain: 1.0,
        channels: 0b1,
        camera_space: false,
    }
}

/// The acceptance criterion in one line: twice as far shakes visibly less.
#[test]
fn further_away_is_a_tremor() {
    let mut impulses = Impulses::default();
    impulses.emit(source(), Vec3::ZERO);
    let near = impulses.heard(Vec3::new(6.0, 0.0, 0.0), listener()).y.abs();
    let far = impulses
        .heard(Vec3::new(12.0, 0.0, 0.0), listener())
        .y
        .abs();
    assert!(near > far, "near {near} is not stronger than far {far}");
    assert!(far > 0.0, "it should still be felt at 12 m, not {far}");
}

/// Inside the radius it is felt whole, past the dissipation not at all.
#[test]
fn the_radius_is_full_and_past_it_is_nothing() {
    let mut impulses = Impulses::default();
    impulses.emit(source(), Vec3::ZERO);
    let inside = impulses.heard(Vec3::new(2.0, 0.0, 0.0), listener()).y;
    let at_source = impulses.heard(Vec3::ZERO, listener()).y;
    assert!(
        (inside - at_source).abs() < 1e-5,
        "it fades inside the radius"
    );
    assert_eq!(
        impulses.heard(Vec3::new(100.0, 0.0, 0.0), listener()),
        Vec3::ZERO,
        "it is heard past its dissipation",
    );
}

/// 🔴 No corner at the radius. A linear falloff drops at full slope the instant the listener
/// crosses it, so a camera walking past that distance changes how hard it is shaken in one frame —
/// which reads as a second, smaller impulse firing. Smoothstep leaves the slope at zero there, and
/// asserting the SHAPE is what separates the two: the first version only asked that near beat far,
/// which a linear falloff satisfies perfectly.
#[test]
fn the_falloff_has_no_corner_at_the_radius() {
    let mut impulses = Impulses::default();
    impulses.emit(source(), Vec3::ZERO);
    let felt = |d: f32| impulses.heard(Vec3::new(d, 0.0, 0.0), listener()).y.abs();
    // Just past the radius the fall has barely begun; a linear one would already be 2 % down.
    let step = felt(5.0) - felt(5.2);
    assert!(
        step < 0.002,
        "it dropped {step} in the first 20 cm past the radius, which is a corner",
    );
}

/// A listener sharing no channel hears nothing — the reason channels exist.
#[test]
fn a_deaf_channel_hears_nothing() {
    let mut impulses = Impulses::default();
    impulses.emit(source(), Vec3::ZERO);
    let elsewhere = ImpulseListener {
        channels: 0b10,
        ..listener()
    };
    assert_eq!(impulses.heard(Vec3::ZERO, elsewhere), Vec3::ZERO);
}

/// 🔴 A finished impulse is DROPPED, not held at its last value. Kept, every landing a player ever
/// made would be walked over for the rest of the session.
#[test]
fn a_finished_impulse_is_dropped() {
    let mut impulses = Impulses::default();
    impulses.emit(source(), Vec3::ZERO);
    impulses.step(0.5);
    assert!(!impulses.is_empty(), "it ended halfway through");
    impulses.step(0.6);
    assert!(impulses.is_empty(), "it outlived its duration");
}
