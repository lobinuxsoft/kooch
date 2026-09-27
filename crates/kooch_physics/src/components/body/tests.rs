//! Tests for [`PhysicsBody`](super::PhysicsBody) and [`Collider`](super::Collider).

/// Any entity — these tests are about the shape, not about who owns it.
fn any_entity() -> kooch_ecs::Entity {
    kooch_ecs::Entity::new(0, 0)
}
use super::*;
use glam::Vec3;

use crate::backend::{BodyKind, CollisionShape};

#[test]
fn defaults_are_a_one_kilo_dynamic_unit_sphere() {
    let body = PhysicsBody::default();
    assert_eq!(body.body_kind(), BodyKind::Dynamic);
    assert_eq!(body.mass, 1.0);
    assert_eq!(
        Collider::default().collision_shape(any_entity(), None),
        Some(CollisionShape::Sphere { radius: 0.5 })
    );
}

#[test]
fn unknown_discriminants_fall_back_instead_of_failing() {
    let body = PhysicsBody {
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
        collider.collision_shape(any_entity(), None),
        Some(CollisionShape::Sphere { .. })
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
    let Some(CollisionShape::Cuboid { half_extents }) =
        collider.collision_shape(any_entity(), None)
    else {
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
        collider.collision_shape(any_entity(), None),
        Some(CollisionShape::Cuboid {
            half_extents: Vec3::splat(2.0)
        })
    );
    collider.shape = SHAPE_CAPSULE;
    assert_eq!(
        collider.collision_shape(any_entity(), None),
        Some(CollisionShape::Capsule {
            radius: 0.25,
            half_height: 1.0
        })
    );
}

/// A scene authored before the rename says `sensor`, and the toggle it meant is `is_trigger`.
#[test]
fn an_old_sensor_is_a_trigger() {
    use kooch_ecs::reflect::{Reflect, ReflectValue};

    let mut collider = Collider::default();
    collider
        .reflect_set("sensor", ReflectValue::Bool(true))
        .unwrap();
    assert!(collider.is_trigger);
}

/// The solver masks are gone, so a scene that still carries them names a field nobody has: the
/// scene loader reports that as a skipped field, not a failed load (#1309).
#[test]
fn a_dropped_solver_mask_is_refused() {
    use kooch_ecs::reflect::{Reflect, ReflectError, ReflectValue};

    let err = Collider::default()
        .reflect_set("solver_memberships", ReflectValue::U32(2))
        .unwrap_err();
    assert!(matches!(err, ReflectError::FieldNotFound(_)));
}

/// 🔴 #1320: a collider is in as many layers as it is ticked into, and it meets whatever any of
/// them meets — the union of their rows, not one row.
#[test]
fn a_collider_meets_every_row_it_is_in() {
    use kooch_core::layers::LayerNames;

    let mut layers = LayerNames::default();
    layers.set(1, "Player");
    layers.set(2, "Scenery");
    // Player meets nothing but itself; Scenery meets Default.
    layers.set_collide(0, 1, false);
    layers.set_collide(1, 2, false);

    let both = Collider {
        layers: 0b110,
        ..Default::default()
    }
    .in_layers(&layers);
    assert_eq!(both.collision_memberships, 0b110, "it is in both layers");
    assert!(
        both.collision_filter & 1 != 0,
        "Scenery meets Default, so a collider in Scenery does too",
    );
}

/// Ticking no layer means meeting nothing, which is a thing an author can want and a thing the
/// default must not quietly become.
#[test]
fn no_layer_meets_nothing() {
    use kooch_core::layers::LayerNames;

    let mut layers = LayerNames::default();
    layers.set_collide(0, 0, true);
    let empty = Collider {
        layers: 0,
        ..Default::default()
    }
    .in_layers(&layers);
    assert_eq!(empty.collision_filter, 0, "an empty mask met something");
}

/// A scene the #1302 editor saved names one layer; a scene older than the matrix carries a
/// membership mask. Both are read as the mask they meant.
#[test]
fn an_older_field_becomes_the_mask() {
    let named = Collider {
        layer: 2,
        ..Default::default()
    };
    assert_eq!(named.layer_mask(), 0b100, "the layer it named, as a bit");

    // 🔴 The lowest bit, not the mask: a pre-matrix project authored "every group except that one",
    // and reading 0xFFFFFFFD literally puts a planet in every layer a volume listens to.
    let pre_matrix = Collider {
        collision_memberships: 0b1010,
        ..Default::default()
    };
    assert_eq!(pre_matrix.layer_mask(), 0b10, "the lowest layer it claimed");

    let everything_but_one = Collider {
        collision_memberships: 0xFFFF_FFFD,
        ..Default::default()
    };
    assert_eq!(
        everything_but_one.layer_mask(),
        kooch_core::layers::DEFAULT_LAYER,
        "a planet ended up in every layer",
    );
}
