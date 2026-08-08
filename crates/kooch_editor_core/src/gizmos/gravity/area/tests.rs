use super::*;
use crate::gizmos::gravity::harness::{draw, reach, shafts};
use glam::{Mat4, Quat};

#[test]
fn an_area_draws_its_box() {
    let field = AreaGravity {
        half_extents: Vec3::splat(5.0),
        falloff: 0.0,
        ..Default::default()
    };
    let corner = Vec3::splat(5.0).length();
    let reach = reach(&draw(&AreaGravityVisualizer, &field, Mat4::IDENTITY));
    assert!(
        (reach - corner).abs() < 0.1,
        "reached {reach}, wanted {corner}",
    );
}

/// The falloff is where the field actually ends, and it is invisible
/// otherwise: a 0.1 m fade and a 5 m fade look identical without it.
#[test]
fn an_area_draws_the_reach_of_its_falloff() {
    let with = AreaGravity {
        half_extents: Vec3::splat(5.0),
        falloff: 5.0,
        ..Default::default()
    };
    let without = AreaGravity {
        falloff: 0.0,
        ..with
    };
    assert!(
        reach(&draw(&AreaGravityVisualizer, &with, Mat4::IDENTITY))
            > reach(&draw(&AreaGravityVisualizer, &without, Mat4::IDENTITY)) + 1.0,
    );
}

/// `direction` is local, so a rotated zone must draw rotated. This is
/// the case that had no way of being seen and dropped things sideways
/// for a reason nobody could point at.
#[test]
fn an_area_turns_with_its_entity() {
    let field = AreaGravity::default();
    let turned = Mat4::from_quat(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
    let shafts = shafts(&draw(&AreaGravityVisualizer, &field, turned));

    assert!(!shafts.is_empty(), "no arrows were drawn");
    // Local -Y turned a quarter turn about +Z points along +X, the same
    // answer `plugin::gravity_at` gives for the same transform.
    for shaft in shafts {
        assert!(
            shaft.abs_diff_eq(Vec3::X, 1e-3),
            "a rotated zone drew its arrow along {shaft}",
        );
    }
}

/// A scaled entity has a bigger zone, the same way the solver scales it.
#[test]
fn an_area_scales_with_its_entity() {
    let field = AreaGravity {
        half_extents: Vec3::splat(5.0),
        falloff: 0.0,
        ..Default::default()
    };
    let plain = reach(&draw(&AreaGravityVisualizer, &field, Mat4::IDENTITY));
    let scaled = reach(&draw(
        &AreaGravityVisualizer,
        &field,
        Mat4::from_scale(Vec3::splat(2.0)),
    ));
    assert!((scaled / plain - 2.0).abs() < 0.05, "{plain} then {scaled}");
}
