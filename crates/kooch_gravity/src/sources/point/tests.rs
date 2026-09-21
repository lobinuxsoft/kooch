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

/// Near the centre the pull holds rather than growing: a field that grew towards a point would go
/// to infinity there and launch things out of the world.
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
fn beyond_the_reach_a_source_contributes_nothing() {
    let source = PointGravity {
        radius: 100.0,
        falloff: 10.0,
        ..Default::default()
    };
    assert_eq!(
        source.acceleration_at(Vec3::ZERO, Vec3::new(0.0, 111.0, 0.0)),
        Vec3::ZERO,
    );
    assert_ne!(
        source.acceleration_at(Vec3::ZERO, Vec3::new(0.0, 99.0, 0.0)),
        Vec3::ZERO,
    );
}

fn fading() -> PointGravity {
    PointGravity {
        strength: 10.0,
        radius: 10.0,
        falloff: 10.0,
    }
}

/// Halfway across the fade is half the pull; past it, nothing.
#[test]
fn a_falloff_fades_past_range() {
    let source = fading();
    let half = source.acceleration_at(Vec3::ZERO, Vec3::new(15.0, 0.0, 0.0));
    assert!((half.length() - 5.0).abs() < 1e-4, "{half}");
    let gone = source.acceleration_at(Vec3::ZERO, Vec3::new(21.0, 0.0, 0.0));
    assert_eq!(gone, Vec3::ZERO);
}

/// Zero is the edge it always was: every scene saved before the field existed loads unchanged.
#[test]
fn no_falloff_is_a_hard_edge() {
    let source = PointGravity {
        falloff: 0.0,
        ..fading()
    };
    assert_eq!(source.influence(10.0), 1.0);
    assert_eq!(source.influence(10.001), 0.0);
}

/// 🔴 A planet saved before `range` folded into `radius` keeps its reach. The old file writes the
/// reference radius first and the reach after it, so reading `range` into `radius` last is what an
/// old planet ends up with — its 25 m, not its 7.
#[test]
fn an_old_planet_keeps_its_reach() {
    use kooch_ecs::reflect::{Reflect, ReflectValue};

    let mut planet = PointGravity::default();
    for (name, value) in [
        ("strength", ReflectValue::F32(19.62)),
        ("radius", ReflectValue::F32(7.0)),
        ("range", ReflectValue::F32(25.0)),
        ("inverse_square", ReflectValue::Bool(true)),
    ] {
        let _ = planet.reflect_set(name, value);
    }
    assert_eq!(planet.radius, 25.0);
    assert_eq!(planet.strength, 19.62);
}

/// Inside the radius the pull is whole, however near the edge — the shape every other bounded
/// source has.
#[test]
fn the_pull_is_whole_inside() {
    let planet = PointGravity {
        strength: 10.0,
        radius: 20.0,
        falloff: 5.0,
    };
    let near = planet.acceleration_at(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0));
    let edge = planet.acceleration_at(Vec3::ZERO, Vec3::new(19.9, 0.0, 0.0));
    assert!((near.length() - 10.0).abs() < 1e-4 && (edge.length() - 10.0).abs() < 1e-4);
}
