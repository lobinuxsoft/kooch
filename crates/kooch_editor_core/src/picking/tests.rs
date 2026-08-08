use super::*;

fn unit_box() -> Aabb {
    Aabb::new(Vec3::splat(-0.5), Vec3::splat(0.5))
}

#[test]
fn a_ray_down_the_axis_hits_a_box_in_front_of_it() {
    let hit = hit_distance(
        unit_box(),
        Mat4::IDENTITY,
        Vec3::new(0.0, 0.0, 5.0),
        Vec3::NEG_Z,
    );
    assert!((hit.unwrap() - 4.5).abs() < 1e-4, "got {hit:?}");
}

#[test]
fn a_ray_that_misses_reports_nothing() {
    assert_eq!(
        hit_distance(
            unit_box(),
            Mat4::IDENTITY,
            Vec3::new(5.0, 5.0, 5.0),
            Vec3::NEG_Z
        ),
        None,
    );
}

/// A box behind the camera is not something the user clicked, even
/// though the infinite line through the cursor passes through it.
#[test]
fn a_box_behind_the_camera_is_not_picked() {
    assert_eq!(
        hit_distance(
            unit_box(),
            Mat4::IDENTITY,
            Vec3::new(0.0, 0.0, 5.0),
            Vec3::Z
        ),
        None,
    );
}

/// Inside the box, the surface the ray reaches is the far one — and it
/// is still a hit, not a miss.
#[test]
fn standing_inside_a_box_still_picks_it() {
    let hit = hit_distance(unit_box(), Mat4::IDENTITY, Vec3::ZERO, Vec3::NEG_Z);
    assert!((hit.unwrap() - 0.5).abs() < 1e-4, "got {hit:?}");
}

/// The whole reason the ray is transformed instead of the box.
///
/// Rotated 45° about Y, the unit box becomes a diamond in the xz
/// plane — |x| + |z| <= 0.707 — while its world-space AABB is the
/// square ±0.707. The corner of that square is empty space, so it has
/// to be aimed at directly: a ray straight down through (0.6, 0.6)
/// lands in the square but outside the diamond.
///
/// Aiming along -Z instead would prove nothing. Both boxes span the
/// same ±0.707 in x, so every such ray agrees.
#[test]
fn a_rotated_box_is_tested_as_itself_not_as_its_world_bounds() {
    let rotated = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_4);
    assert_eq!(
        hit_distance(unit_box(), rotated, Vec3::new(0.6, 5.0, 0.6), Vec3::NEG_Y),
        None,
        "the inflated world-space box would have swallowed this corner",
    );
    assert!(
        hit_distance(unit_box(), rotated, Vec3::new(0.3, 5.0, 0.3), Vec3::NEG_Y).is_some(),
        "a ray through the real box must still hit",
    );
}

/// Scale must not distort the reported distance, or a scaled-up entity
/// would always claim to be nearer than an unscaled one beside it.
#[test]
fn distance_stays_in_world_units_under_scale() {
    let scaled = Mat4::from_scale(Vec3::splat(2.0));
    // Box scaled to ±1 by the transform: the surface is at z = 1, so
    // the ray from z = 5 travels 4 world units.
    let hit = hit_distance(unit_box(), scaled, Vec3::new(0.0, 0.0, 5.0), Vec3::NEG_Z);
    assert!((hit.unwrap() - 4.0).abs() < 1e-4, "got {hit:?}");
}

/// A degenerate transform (zero scale) has no inverse. Testing against
/// the resulting NaNs would pick unpredictably.
#[test]
fn an_entity_with_no_volume_is_not_picked() {
    let collapsed = Mat4::from_scale(Vec3::ZERO);
    assert_eq!(
        hit_distance(unit_box(), collapsed, Vec3::new(0.0, 0.0, 5.0), Vec3::NEG_Z),
        None,
    );
}
