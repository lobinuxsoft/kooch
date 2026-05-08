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
