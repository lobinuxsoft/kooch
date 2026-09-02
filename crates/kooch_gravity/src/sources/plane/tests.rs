use super::*;

#[test]
fn a_plane_pulls_along_its_normal() {
    let field = PlaneGravity::default();
    let accel = field.acceleration_at_local(Vec3::new(300.0, 5.0, -900.0));
    assert_eq!(accel, Vec3::new(0.0, -9.81, 0.0));
}

/// Unbounded across the plane is the whole difference from an area: a
/// floor does not stop because a body walked far enough sideways.
#[test]
fn distance_sideways_changes_nothing() {
    let field = PlaneGravity::default();
    let near = field.acceleration_at_local(Vec3::new(0.0, 5.0, 0.0));
    let far = field.acceleration_at_local(Vec3::new(1.0e6, 5.0, -1.0e6));
    assert_eq!(near, far);
}

/// A plane with no thickness that pulled from both sides would trap a
/// body in it, pushed back from whichever side it reached.
#[test]
fn nothing_below_the_plane() {
    let field = PlaneGravity::default();
    assert_eq!(
        field.acceleration_at_local(Vec3::new(0.0, -1.0, 0.0)),
        Vec3::ZERO
    );
}

#[test]
fn the_field_fades_across_the_falloff() {
    let field = PlaneGravity {
        range: 10.0,
        falloff: 4.0,
        ..Default::default()
    };
    assert!((field.influence_at_local(Vec3::new(0.0, 10.0, 0.0)) - 1.0).abs() < 1e-6);
    assert!((field.influence_at_local(Vec3::new(0.0, 12.0, 0.0)) - 0.5).abs() < 1e-6);
    assert_eq!(field.influence_at_local(Vec3::new(0.0, 14.1, 0.0)), 0.0);
}

/// Zero or less is unlimited, and `falloff` then never applies.
#[test]
fn an_unlimited_plane_never_fades() {
    let field = PlaneGravity {
        range: 0.0,
        falloff: 4.0,
        ..Default::default()
    };
    assert_eq!(field.influence_at_local(Vec3::new(0.0, 1.0e6, 0.0)), 1.0);
}

/// A normal that cannot be normalised has no plane to be a side of.
#[test]
fn a_zero_normal_is_inert() {
    let field = PlaneGravity {
        normal: Vec3::ZERO,
        ..Default::default()
    };
    assert_eq!(field.acceleration_at_local(Vec3::Y), Vec3::ZERO);
}
