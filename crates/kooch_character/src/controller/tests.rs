use super::*;
use glam::Vec3;

#[test]
fn flat_ground_is_standable() {
    assert!(CharacterController::default().stands_on(Vec3::Y, Vec3::Y));
}

/// A wall is found by the probe and is still not a floor. Without this a
/// character pressed against a cliff can jump off it forever.
#[test]
fn a_wall_is_not_ground() {
    assert!(!CharacterController::default().stands_on(Vec3::X, Vec3::Y));
}

#[test]
fn the_limit_is_where_it_says() {
    let controller = CharacterController {
        max_slope: 45.0,
        ..Default::default()
    };
    let tilt = |degrees: f32| {
        let a = degrees.to_radians();
        Vec3::new(a.sin(), a.cos(), 0.0)
    };
    assert!(controller.stands_on(tilt(44.0), Vec3::Y));
    assert!(!controller.stands_on(tilt(46.0), Vec3::Y));
}

/// Every term is written against the local up, so a ceiling on a planet
/// is a floor once you are standing on the other side of it.
#[test]
fn the_slope_is_measured_from_local_up() {
    let controller = CharacterController::default();
    assert!(controller.stands_on(Vec3::NEG_Y, Vec3::NEG_Y));
    assert!(!controller.stands_on(Vec3::Y, Vec3::NEG_Y));
}

/// A degenerate normal is what a sweep returns when it found nothing.
#[test]
fn no_normal_is_no_ground() {
    assert!(!CharacterController::default().stands_on(Vec3::ZERO, Vec3::Y));
}
