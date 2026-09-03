use super::*;

/// The claim the bug was about: it points where it is steered.
#[test]
fn it_faces_where_it_is_steered() {
    let facing = Vec3::new(0.0, 0.0, -1.0);
    let turned = wanted(Vec3::Y, facing, Quat::IDENTITY) * Vec3::NEG_Z;
    assert!(
        turned.dot(facing) > 0.99,
        "should look along {facing}, looked along {turned}",
    );
}

/// Steering into a hill turns along the hill, not into it.
#[test]
fn a_facing_is_flattened() {
    let up = Vec3::Y;
    let steered = Vec3::new(0.0, 0.8, -0.6);
    let turned = wanted(up, steered, Quat::IDENTITY);
    assert!((turned * Vec3::Y).dot(up) > 0.999, "should stand on up");
    assert!((turned * Vec3::NEG_Z).dot(up).abs() < 1e-3, "and look flat");
}

/// Upside down on a planet is an ordinary answer, not a special case.
#[test]
fn it_stands_on_any_up() {
    let up = Vec3::NEG_Y;
    let turned = wanted(up, Vec3::X, Quat::IDENTITY);
    assert!((turned * Vec3::Y).dot(up) > 0.999);
}

/// Nothing to look along keeps the yaw it already has. Snapping to
/// an arbitrary perpendicular is a character that spins when the
/// stick is released.
#[test]
fn no_facing_keeps_the_yaw() {
    let current = Quat::from_rotation_y(1.2);
    for steered in [Vec3::ZERO, Vec3::Y * 3.0] {
        let turned = wanted(Vec3::Y, steered, current);
        assert!(turned.abs_diff_eq(current, 1e-5), "{turned} vs {current}");
    }
}

/// Standing up is not optional. Without this a character with no
/// `Facing` never rights itself, which is the regression the planet
/// test caught.
#[test]
fn no_facing_still_stands_up() {
    let up = Vec3::X;
    let turned = wanted(up, Vec3::ZERO, Quat::IDENTITY);
    assert!((turned * Vec3::Y).dot(up) > 0.999, "should stand on {up}");
}

/// A slow frame turns the whole way and stops, rather than past it.
#[test]
fn a_long_step_cannot_overshoot() {
    let target = Quat::from_rotation_y(std::f32::consts::PI * 0.5);
    let turned = towards(Quat::IDENTITY, target, 10.0, 1.0);
    assert!(turned.abs_diff_eq(target, 1e-5), "{turned} vs {target}");
}
