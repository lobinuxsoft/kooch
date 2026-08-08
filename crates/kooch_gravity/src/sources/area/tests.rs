use super::*;

#[test]
fn an_area_is_at_full_strength_inside_its_box() {
    let area = AreaGravity::default();
    assert_eq!(area.influence_at_local(Vec3::ZERO), 1.0);
    assert_eq!(area.influence_at_local(Vec3::new(4.9, 0.0, 0.0)), 1.0);
}

/// A body crossing the boundary should not change direction between
/// one step and the next.
#[test]
fn an_area_fades_across_its_falloff() {
    let area = AreaGravity {
        falloff: 2.0,
        ..Default::default()
    };
    let half_way = area.influence_at_local(Vec3::new(6.0, 0.0, 0.0));
    assert!(
        (half_way - 0.5).abs() < 1e-3,
        "one metre past a 5 m box with 2 m falloff should be half: {half_way}",
    );
    assert_eq!(area.influence_at_local(Vec3::new(7.1, 0.0, 0.0)), 0.0);
}

#[test]
fn a_hard_edged_area_stops_at_its_boundary() {
    let area = AreaGravity {
        falloff: 0.0,
        ..Default::default()
    };
    assert_eq!(area.influence_at_local(Vec3::new(5.1, 0.0, 0.0)), 0.0);
    assert_eq!(area.influence_at_local(Vec3::new(4.9, 0.0, 0.0)), 1.0);
}

/// A direction mid-edit passes through zero, and a normalise of zero
/// is a NaN that outlives the typo.
#[test]
fn a_degenerate_direction_applies_nothing() {
    let area = AreaGravity {
        direction: Vec3::ZERO,
        ..Default::default()
    };
    assert_eq!(area.acceleration_at_local(Vec3::ZERO), Vec3::ZERO);
}
