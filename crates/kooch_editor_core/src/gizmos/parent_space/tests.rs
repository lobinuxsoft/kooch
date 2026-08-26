use super::*;

/// A parent yawed 90° about Y has its local +X pointing along world
/// −Z, so world +X reads as local +Z from inside it. Dragging the
/// world-X handle on a child must move it along that, or the entity
/// slides down an axis the user did not grab.
#[test]
fn a_translation_follows_the_parents_rotation() {
    let parent = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2);
    let local = translation_to_parent_space(parent.inverse(), Vec3::X);

    assert!(
        (local - Vec3::Z).length() < 1e-5,
        "expected world +X to read as local +Z, got {local:?}",
    );
    // Guards the sign: negating it must not also pass, or the test
    // would survive the axis flipping.
    assert!((local - Vec3::NEG_Z).length() > 1.0);
}

/// A parent scaled by two halves the local delta: moving one world
/// unit is half a unit in a space where everything is twice as big.
#[test]
fn a_translation_divides_by_the_parents_scale() {
    let parent = Mat4::from_scale(Vec3::splat(2.0));
    let local = translation_to_parent_space(parent.inverse(), Vec3::X);

    assert!(
        (local - Vec3::X * 0.5).length() < 1e-5,
        "expected half a unit, got {local:?}",
    );
}

/// The parent's position must not leak into a translation delta — it
/// is a direction, not a point. This is what `transform_vector3`
/// buys over `transform_point3`, and getting it wrong offsets every
/// drag by the parent's position.
#[test]
fn a_translation_ignores_where_the_parent_is() {
    let far_away = Mat4::from_translation(Vec3::new(100.0, -50.0, 7.0));
    let local = translation_to_parent_space(far_away.inverse(), Vec3::X);

    assert!(
        (local - Vec3::X).length() < 1e-5,
        "a pure translation on the parent must not change the delta, got {local:?}",
    );
}

/// With no parent rotation, a rotation delta passes through unchanged.
#[test]
fn a_rotation_under_an_unrotated_parent_is_unchanged() {
    let delta = Quat::from_rotation_x(0.5);
    let converted = rotation_to_parent_space(Mat4::IDENTITY, delta);

    assert!(
        (converted * delta.inverse()).angle_between(Quat::IDENTITY) < 1e-5,
        "expected the delta to survive untouched",
    );
}

/// A yaw applied in world space, seen from a parent rolled 90° about
/// Z, becomes a rotation about a different local axis. The angle is
/// preserved; only the axis moves.
#[test]
fn a_rotation_is_conjugated_by_the_parents_rotation() {
    let parent = Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2);
    let delta = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);
    let converted = rotation_to_parent_space(parent.inverse(), delta);

    assert!(
        (converted.angle_between(Quat::IDENTITY) - std::f32::consts::FRAC_PI_4).abs() < 1e-4,
        "the angle must survive the change of basis",
    );

    let (axis, _) = converted.to_axis_angle();
    assert!(
        axis.dot(Vec3::Y).abs() < 0.99,
        "the axis must have moved out of world Y, got {axis:?}",
    );
}

/// A drag on an unparented entity must be left exactly alone, since
/// its local space is world space.
#[test]
fn an_entity_with_no_parent_needs_no_conversion() {
    let resources = Resources::new();
    assert!(parent_world_to_local(&resources, Entity::new(0, 0)).is_none());
}
