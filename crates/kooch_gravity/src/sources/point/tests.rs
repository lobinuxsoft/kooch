use super::*;

#[test]
fn a_point_source_pulls_towards_itself() {
    let source = PointGravity::default();
    let accel = source.acceleration_at(Vec3::ZERO, Vec3::new(50.0, 0.0, 0.0));
    assert!(
        accel.x < 0.0,
        "should pull back towards the origin: {accel}"
    );
    assert!((accel.length() - 9.81).abs() < 1e-3, "{accel}");
}

/// The strength is quoted at the radius, so that is where it holds
/// exactly — which is what makes it an authorable number.
#[test]
fn the_strength_is_exact_at_the_radius() {
    let source = PointGravity::default();
    let at_radius = source.acceleration_at(Vec3::ZERO, Vec3::new(0.0, 50.0, 0.0));
    assert!((at_radius.length() - 9.81).abs() < 1e-3);

    let further = source.acceleration_at(Vec3::ZERO, Vec3::new(0.0, 100.0, 0.0));
    assert!(
        (further.length() - 9.81 / 4.0).abs() < 1e-3,
        "twice the distance should be a quarter the pull, got {}",
        further.length(),
    );
}

/// Inside the reference radius the pull holds rather than growing.
/// Unclamped it goes to infinity at the centre, which launches things
/// out of the world.
#[test]
fn the_pull_does_not_grow_without_bound_near_the_centre() {
    let source = PointGravity::default();
    let close = source.acceleration_at(Vec3::ZERO, Vec3::new(0.0, 0.001, 0.0));
    assert!(close.length() <= 9.81 + 1e-3, "{}", close.length());
    assert!(close.is_finite());
}

/// Exactly at the centre there is no direction to pull in.
#[test]
fn a_body_at_the_centre_is_pulled_nowhere() {
    let accel = PointGravity::default().acceleration_at(Vec3::ZERO, Vec3::ZERO);
    assert_eq!(accel, Vec3::ZERO);
}

/// The cutoff is what keeps a galaxy of sources from costing every
/// body every step.
#[test]
fn beyond_the_range_a_source_contributes_nothing() {
    let source = PointGravity {
        range: 100.0,
        ..Default::default()
    };
    assert_eq!(
        source.acceleration_at(Vec3::ZERO, Vec3::new(0.0, 101.0, 0.0)),
        Vec3::ZERO,
    );
    assert_ne!(
        source.acceleration_at(Vec3::ZERO, Vec3::new(0.0, 99.0, 0.0)),
        Vec3::ZERO,
    );
}

#[test]
fn a_constant_point_source_does_not_fall_off() {
    let source = PointGravity {
        inverse_square: false,
        ..Default::default()
    };
    let near = source.acceleration_at(Vec3::ZERO, Vec3::new(0.0, 10.0, 0.0));
    let far = source.acceleration_at(Vec3::ZERO, Vec3::new(0.0, 400.0, 0.0));
    assert!((near.length() - far.length()).abs() < 1e-3);
}
