use super::*;

const EPS: f32 = 1e-5;

fn unit_box() -> Aabb {
    Aabb::new(Vec3::ZERO, Vec3::splat(1.0))
}

#[test]
fn center_and_extents() {
    let b = Aabb::new(Vec3::ZERO, Vec3::new(2.0, 4.0, 6.0));
    assert_eq!(b.center(), Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(b.extents(), Vec3::new(1.0, 2.0, 3.0));
}

#[test]
fn distance_squared_inside_is_zero() {
    let b = unit_box();
    assert!(b.distance_squared(Vec3::splat(0.5)) < EPS);
}

#[test]
fn distance_squared_outside_corner() {
    let b = unit_box();
    // Closest point on the box is (0,0,0); distance² = 9 + 16 + 0 = 25.
    let d2 = b.distance_squared(Vec3::new(-3.0, -4.0, 0.0));
    assert!((d2 - 25.0).abs() < EPS);
}

#[test]
fn ray_intersect_hit_from_outside() {
    let b = unit_box();
    // Ray from (-1, 0.5, 0.5) towards +X — must hit at t = 1.
    let hit = b.ray_intersect(Vec3::new(-1.0, 0.5, 0.5), Vec3::X);
    let (t_near, t_far) = hit.expect("expected hit");
    assert!((t_near - 1.0).abs() < EPS);
    assert!((t_far - 2.0).abs() < EPS);
}

#[test]
fn ray_intersect_miss() {
    let b = unit_box();
    // Ray parallel to +Y at x = 2 — outside the slab on x.
    assert!(
        b.ray_intersect(Vec3::new(2.0, -1.0, 0.5), Vec3::Y)
            .is_none()
    );
}

#[test]
fn ray_intersect_origin_inside() {
    let b = unit_box();
    let hit = b.ray_intersect(Vec3::splat(0.5), Vec3::X);
    let (t_near, t_far) = hit.expect("expected hit from inside");
    // t_near is negative (the back wall is behind), t_far > 0.
    assert!(t_near < 0.0);
    assert!((t_far - 0.5).abs() < EPS);
}

#[test]
fn expand_grows_box() {
    let mut b = Aabb::EMPTY;
    b.expand(Vec3::new(1.0, 2.0, 3.0));
    b.expand(Vec3::new(-1.0, 0.0, 4.0));
    assert_eq!(b.min, Vec3::new(-1.0, 0.0, 3.0));
    assert_eq!(b.max, Vec3::new(1.0, 2.0, 4.0));
}

#[test]
fn empty_sentinel_is_empty() {
    assert!(Aabb::EMPTY.is_empty());
    assert!(Aabb::default().is_empty());
    assert!(!unit_box().is_empty());
}

#[test]
fn union_covers_both() {
    let a = Aabb::new(Vec3::ZERO, Vec3::splat(1.0));
    let b = Aabb::new(Vec3::new(2.0, 0.0, 0.0), Vec3::new(3.0, 1.0, 1.0));
    let u = a.union(&b);
    assert_eq!(u.min, Vec3::ZERO);
    assert_eq!(u.max, Vec3::new(3.0, 1.0, 1.0));
}
