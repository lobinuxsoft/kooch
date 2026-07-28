//! Tests for [`RigidBody`](super::RigidBody) and [`Collider`](super::Collider).

use super::*;
use glam::Vec3;

use crate::backend::{BodyKind, CollisionShape};

#[test]
fn defaults_are_a_one_kilo_dynamic_unit_sphere() {
    let body = RigidBody::default();
    assert_eq!(body.body_kind(), BodyKind::Dynamic);
    assert_eq!(body.mass, 1.0);
    assert_eq!(
        Collider::default().collision_shape(),
        CollisionShape::Sphere { radius: 0.5 }
    );
}

#[test]
fn unknown_discriminants_fall_back_instead_of_failing() {
    let body = RigidBody {
        kind: 99,
        mass: 1.0,
        ..Default::default()
    };
    assert_eq!(body.body_kind(), BodyKind::Dynamic);

    let collider = Collider {
        shape: 99,
        ..Default::default()
    };
    assert!(matches!(
        collider.collision_shape(),
        CollisionShape::Sphere { .. }
    ));
}

/// A shape being typed into the Inspector passes through zero, and a
/// zero-sized collider poisons the solver long after the typo.
#[test]
fn degenerate_dimensions_are_clamped() {
    let collider = Collider {
        shape: SHAPE_CUBOID,
        half_extents: Vec3::ZERO,
        ..Default::default()
    };
    let CollisionShape::Cuboid { half_extents } = collider.collision_shape() else {
        panic!("expected a cuboid");
    };
    assert!(half_extents.min_element() > 0.0);
}

/// Switching shape must not destroy the other variant's parameters —
/// the Inspector shows them all at once.
#[test]
fn switching_shape_keeps_the_other_parameters() {
    let mut collider = Collider {
        shape: SHAPE_CAPSULE,
        radius: 0.25,
        half_extents: Vec3::splat(2.0),
        half_height: 1.0,
        center: Vec3::ZERO,
        ..Default::default()
    };
    collider.shape = SHAPE_CUBOID;
    assert_eq!(
        collider.collision_shape(),
        CollisionShape::Cuboid {
            half_extents: Vec3::splat(2.0)
        }
    );
    collider.shape = SHAPE_CAPSULE;
    assert_eq!(
        collider.collision_shape(),
        CollisionShape::Capsule {
            radius: 0.25,
            half_height: 1.0
        }
    );
}
