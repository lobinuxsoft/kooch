use glam::{Quat, Vec3};

use crate::backend::{BodyDesc, CollisionShape, PhysicsBackend};

use super::backend::RapierBackend;

#[test]
fn new_backend_is_empty() {
    let backend = RapierBackend::new();
    assert_eq!(backend.body_count(), 0);
    assert_eq!(backend.gravity(), Vec3::new(0.0, -9.81, 0.0));
}

#[test]
fn add_body_increments_count_and_returns_live_handle() {
    let mut backend = RapierBackend::new();
    let handle = backend.add_body(BodyDesc::dynamic(
        CollisionShape::Sphere { radius: 1.0 },
        1.0,
    ));
    assert_eq!(backend.body_count(), 1);
    assert!(backend.contains(handle));
    assert!(backend.get_transform(handle).is_some());
}

#[test]
fn remove_body_invalidates_handle() {
    let mut backend = RapierBackend::new();
    let handle = backend.add_body(BodyDesc::dynamic(
        CollisionShape::Sphere { radius: 1.0 },
        1.0,
    ));
    backend.remove_body(handle);
    assert_eq!(backend.body_count(), 0);
    assert!(!backend.contains(handle));
    assert!(backend.get_transform(handle).is_none());
}

#[test]
fn dynamic_body_falls_under_gravity() {
    let mut backend = RapierBackend::new();
    let handle = backend.add_body(BodyDesc::dynamic(
        CollisionShape::Sphere { radius: 0.5 },
        1.0,
    ));
    let (initial_pos, _) = backend.get_transform(handle).unwrap();
    // Step ~16ms ten times (~160ms total) — body should fall noticeably.
    for _ in 0..10 {
        backend.step(0.016);
    }
    let (final_pos, _) = backend.get_transform(handle).unwrap();
    assert!(
        final_pos.y < initial_pos.y - 0.1,
        "body should have fallen at least 10 cm under gravity, got {} -> {}",
        initial_pos.y,
        final_pos.y,
    );
}

#[test]
fn static_body_does_not_move() {
    let mut backend = RapierBackend::new();
    let handle = backend.add_body(BodyDesc::static_at(
        CollisionShape::Cuboid {
            half_extents: Vec3::splat(1.0),
        },
        Vec3::new(5.0, 0.0, 0.0),
    ));
    for _ in 0..30 {
        backend.step(0.016);
    }
    let (pos, _) = backend.get_transform(handle).unwrap();
    assert_eq!(pos, Vec3::new(5.0, 0.0, 0.0));
}

#[test]
fn dynamic_body_lands_on_static_floor() {
    let mut backend = RapierBackend::new();
    // Floor: large flat box at y = -0.5 (top surface at y = 0).
    let _floor = backend.add_body(BodyDesc::static_at(
        CollisionShape::Cuboid {
            half_extents: Vec3::new(10.0, 0.5, 10.0),
        },
        Vec3::new(0.0, -0.5, 0.0),
    ));
    // Sphere: radius 0.5 at y = 5.
    let mut desc = BodyDesc::dynamic(CollisionShape::Sphere { radius: 0.5 }, 1.0);
    desc.position = Vec3::new(0.0, 5.0, 0.0);
    let sphere = backend.add_body(desc);

    // Simulate ~2 seconds.
    for _ in 0..120 {
        backend.step(0.016);
    }

    let (pos, _) = backend.get_transform(sphere).unwrap();
    // Sphere should rest on top of the floor — center at ~y = 0.5
    // (radius). Allow some tolerance for solver settling + restitution.
    assert!(
        pos.y > 0.0 && pos.y < 1.0,
        "sphere should rest on floor, got y = {}",
        pos.y,
    );
}

#[test]
fn ray_hits_static_body() {
    let mut backend = RapierBackend::new();
    let body = backend.add_body(BodyDesc::static_at(
        CollisionShape::Sphere { radius: 1.0 },
        Vec3::new(0.0, 0.0, 5.0),
    ));
    // Ray from origin shooting +Z.
    let hit = backend
        .query_ray(Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0), 100.0)
        .expect("ray should hit the sphere at z=5");
    assert_eq!(hit.body, body);
    // Front of sphere is at z = 4, so t ≈ 4.
    assert!((hit.t - 4.0).abs() < 0.1);
}

#[test]
fn ray_misses_when_no_geometry() {
    let backend = RapierBackend::new();
    let hit = backend.query_ray(Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0), 100.0);
    assert!(hit.is_none());
}

#[test]
fn set_transform_teleports_body() {
    let mut backend = RapierBackend::new();
    let handle = backend.add_body(BodyDesc::dynamic(
        CollisionShape::Sphere { radius: 0.5 },
        1.0,
    ));
    backend.set_transform(handle, Vec3::new(7.0, 8.0, 9.0), Quat::IDENTITY);
    let (pos, _) = backend.get_transform(handle).unwrap();
    assert_eq!(pos, Vec3::new(7.0, 8.0, 9.0));
}

#[test]
fn linear_velocity_round_trip() {
    let mut backend = RapierBackend::new();
    let handle = backend.add_body(BodyDesc::dynamic(
        CollisionShape::Sphere { radius: 0.5 },
        1.0,
    ));
    let v = Vec3::new(1.0, 2.0, 3.0);
    backend.set_linear_velocity(handle, v);
    assert_eq!(backend.linear_velocity(handle), Some(v));
}

#[test]
fn stale_handle_after_remove_is_safe() {
    let mut backend = RapierBackend::new();
    let handle = backend.add_body(BodyDesc::dynamic(
        CollisionShape::Sphere { radius: 0.5 },
        1.0,
    ));
    backend.remove_body(handle);
    // No crash, no spurious data.
    assert!(backend.get_transform(handle).is_none());
    backend.set_transform(handle, Vec3::ONE, Quat::IDENTITY); // no-op
    backend.set_linear_velocity(handle, Vec3::ONE); // no-op
    assert!(backend.linear_velocity(handle).is_none());
}

#[test]
fn gravity_override_takes_effect() {
    let mut backend = RapierBackend::new();
    backend.set_gravity(Vec3::new(0.0, 9.81, 0.0)); // upward
    let handle = backend.add_body(BodyDesc::dynamic(
        CollisionShape::Sphere { radius: 0.5 },
        1.0,
    ));
    let (initial, _) = backend.get_transform(handle).unwrap();
    for _ in 0..10 {
        backend.step(0.016);
    }
    let (final_pos, _) = backend.get_transform(handle).unwrap();
    assert!(
        final_pos.y > initial.y + 0.1,
        "inverted gravity should make body rise, got {} -> {}",
        initial.y,
        final_pos.y,
    );
}

// -- Compound colliders (#612) ------------------------------------------

/// One body, several shapes — what Unity calls a compound collider. The
/// alternative, one body per collider held together by the transform
/// hierarchy, is what no engine supports.
#[test]
fn a_body_can_carry_several_shapes() {
    let mut backend = RapierBackend::new();
    let body = backend.add_body(BodyDesc::dynamic(
        CollisionShape::Sphere { radius: 0.5 },
        1.0,
    ));
    assert_eq!(
        backend.collider_count(body),
        Some(1),
        "the body's own shape"
    );

    let attached = backend
        .attach_collider(
            body,
            CollisionShape::Cuboid {
                half_extents: Vec3::splat(0.25),
            },
            Vec3::new(0.0, 1.0, 0.0),
            Quat::IDENTITY,
        )
        .expect("attaching to a live body succeeds");

    assert_eq!(backend.collider_count(body), Some(2));
    assert_eq!(backend.body_count(), 1, "still one body, not two");

    backend.detach_collider(attached);
    assert_eq!(
        backend.collider_count(body),
        Some(1),
        "detaching a shape leaves the body and its own shape alone",
    );
    assert_eq!(backend.body_count(), 1);
}

/// A child entity contributing a collider can be rotated relative to the
/// body. Dropping that would silently axis-align every attached shape.
///
/// Probed with a ray rather than by reading the shape back: the rotation
/// only matters because it changes what the shape occupies, and a ray is
/// the observable form of that.
#[test]
fn an_attached_shape_keeps_its_rotation() {
    /// Fires at a point that a Y-aligned capsule cannot reach, but one
    /// tilted 45° about Z passes straight through.
    fn hits_tilted_arm(rotation: Quat) -> bool {
        let mut backend = RapierBackend::new();
        // A token shape well away from the probe, so only the attached
        // capsule can answer the ray.
        let body = backend.add_body(BodyDesc::static_at(
            CollisionShape::Sphere { radius: 0.05 },
            Vec3::ZERO,
        ));
        backend
            .attach_collider(
                body,
                CollisionShape::Capsule {
                    radius: 0.2,
                    half_height: 1.0,
                },
                Vec3::ZERO,
                rotation,
            )
            .expect("attaches");

        backend
            .query_ray(Vec3::new(0.7, 0.7, -5.0), Vec3::Z, 10.0)
            .is_some()
    }

    let tilted = Quat::from_rotation_z(-std::f32::consts::FRAC_PI_4);
    assert!(
        hits_tilted_arm(tilted),
        "a capsule tilted toward (0.7, 0.7) should be hit there",
    );
    assert!(
        !hits_tilted_arm(Quat::IDENTITY),
        "an upright capsule reaches only 0.2 from the Y axis; \
         if this hits, the rotation was ignored and the test proves nothing",
    );
}

#[test]
fn attaching_to_a_stale_body_returns_none() {
    let mut backend = RapierBackend::new();
    let body = backend.add_body(BodyDesc::dynamic(
        CollisionShape::Sphere { radius: 1.0 },
        1.0,
    ));
    backend.remove_body(body);

    assert!(
        backend
            .attach_collider(
                body,
                CollisionShape::Sphere { radius: 1.0 },
                Vec3::ZERO,
                Quat::IDENTITY,
            )
            .is_none(),
    );
    assert_eq!(backend.collider_count(body), None);
}

/// Removing the body takes its attached shapes with it — otherwise the
/// handles would outlive what they point at.
#[test]
fn removing_a_body_takes_its_attached_shapes() {
    let mut backend = RapierBackend::new();
    let body = backend.add_body(BodyDesc::dynamic(
        CollisionShape::Sphere { radius: 0.5 },
        1.0,
    ));
    backend
        .attach_collider(
            body,
            CollisionShape::Sphere { radius: 0.5 },
            Vec3::X,
            Quat::IDENTITY,
        )
        .expect("attaches");

    backend.remove_body(body);
    assert_eq!(backend.body_count(), 0);
    assert!(!backend.contains(body));
    assert_eq!(
        backend.collider_count(body),
        None,
        "no shape outlived its body",
    );
}
