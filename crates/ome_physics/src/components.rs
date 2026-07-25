//! ECS components describing an entity's physical intent.
//!
//! These say *what* an entity is physically — a dynamic body, a sphere
//! of radius 0.5 — never *how* it is simulated. Nothing here mentions
//! the backend, so swapping the solver (wgrapier, when GPU rigid bodies
//! land) is a change inside [`crate::rapier_backend`], not a rewrite of
//! every scene ever authored.
//!
//! # Why flat fields instead of enums
//!
//! Reflection has no enum representation — [`ReflectValue`] covers
//! scalars, vectors and asset references — so a variant is a `u32`
//! discriminant with labelled `choices`, and the parameters of every
//! variant sit side by side. The alternative was blocking physics on a
//! reflection feature; the shape of the data is the same either way,
//! and [`Collider::shape`] resolves it back to a typed
//! [`CollisionShape`] at the seam.
//!
//! [`ReflectValue`]: ome_ecs::reflect::ReflectValue

use glam::Vec3;

use ome_ecs::Reflect;
use ome_ecs::component::Component;
use ome_ecs::reflect::{FieldChoice, FieldCondition};

use crate::backend::{BodyKind, CollisionShape};

// ---------------------------------------------------------------------------
// RigidBody
// ---------------------------------------------------------------------------

/// Solver-driven: gravity and collisions move it.
pub const KIND_DYNAMIC: u32 = 0;
/// Author-driven: you set its transform, and it pushes dynamic bodies.
pub const KIND_KINEMATIC: u32 = 1;
/// Immovable: nothing moves it, it stops everything else.
pub const KIND_STATIC: u32 = 2;

/// Labels for the `kind` dropdown in the Inspector.
pub static KIND_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "Dynamic",
        value: KIND_DYNAMIC as i64,
    },
    FieldChoice {
        label: "Kinematic",
        value: KIND_KINEMATIC as i64,
    },
    FieldChoice {
        label: "Static",
        value: KIND_STATIC as i64,
    },
];

/// Marks an entity as participating in the physics simulation.
///
/// Pairs with a [`Collider`]; an entity carrying only this gets a unit
/// sphere, so a half-authored entity falls rather than panics.
///
/// # Default
///
/// A dynamic body of 1 kg.
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Physics")]
pub struct RigidBody {
    /// How the solver treats this body. One of the `KIND_*` constants.
    #[reflect(choices = KIND_CHOICES)]
    pub kind: u32,
    /// Mass in kilograms. Ignored by static and kinematic bodies.
    pub mass: f32,
}

impl Default for RigidBody {
    fn default() -> Self {
        Self {
            kind: KIND_DYNAMIC,
            mass: 1.0,
        }
    }
}

impl Component for RigidBody {}

impl RigidBody {
    /// The backend body kind, defaulting to dynamic for a discriminant
    /// outside the known set — a scene authored by a newer editor stays
    /// loadable rather than failing.
    pub fn body_kind(&self) -> BodyKind {
        match self.kind {
            KIND_KINEMATIC => BodyKind::Kinematic,
            KIND_STATIC => BodyKind::Static,
            _ => BodyKind::Dynamic,
        }
    }
}

// ---------------------------------------------------------------------------
// Collider
// ---------------------------------------------------------------------------

/// Ball of radius `radius`.
pub const SHAPE_SPHERE: u32 = 0;
/// Box of half-extents `half_extents`.
pub const SHAPE_CUBOID: u32 = 1;
/// Capsule along local Y: `radius` plus `half_height` excluding caps.
pub const SHAPE_CAPSULE: u32 = 2;

/// Labels for the `shape` dropdown in the Inspector.
pub static SHAPE_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "Sphere",
        value: SHAPE_SPHERE as i64,
    },
    FieldChoice {
        label: "Cuboid",
        value: SHAPE_CUBOID as i64,
    },
    FieldChoice {
        label: "Capsule",
        value: SHAPE_CAPSULE as i64,
    },
];

/// Which shapes read `radius`: the ball and the capsule.
///
/// Beside the `SHAPE_*` constants on purpose — someone adding a shape is
/// already editing here, and a condition kept in another file is a
/// condition that goes stale.
pub static RADIUS_WHEN: FieldCondition = FieldCondition {
    field: "shape",
    values: &[SHAPE_SPHERE as i64, SHAPE_CAPSULE as i64],
};

/// Which shapes read `half_extents`: only the box.
pub static HALF_EXTENTS_WHEN: FieldCondition = FieldCondition {
    field: "shape",
    values: &[SHAPE_CUBOID as i64],
};

/// Which shapes read `half_height`: only the capsule.
pub static HALF_HEIGHT_WHEN: FieldCondition = FieldCondition {
    field: "shape",
    values: &[SHAPE_CAPSULE as i64],
};

/// The collision geometry attached to a body.
///
/// Named for what it becomes rather than for its geometry: a collider is
/// eventually geometry *plus* material and filtering (friction,
/// restitution, sensor flag, collision groups — #137), while
/// [`CollisionShape`] stays the pure geometry the backend consumes.
///
/// Only the fields belonging to the selected `shape` are read, and only
/// those are *shown* — see the `*_WHEN` conditions above. The rest keep
/// whatever they were, so switching shape back and forth does not lose the
/// other variant's parameters. Hiding is display only: every field is
/// still stored, still serialised, still round-trips through a scene.
///
/// # Default
///
/// A unit sphere.
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Physics")]
pub struct Collider {
    /// Which geometry to use. One of the `SHAPE_*` constants.
    #[reflect(choices = SHAPE_CHOICES)]
    pub shape: u32,
    /// Sphere and capsule radius.
    #[reflect(shown_when = RADIUS_WHEN)]
    pub radius: f32,
    /// Cuboid half-extents.
    #[reflect(shown_when = HALF_EXTENTS_WHEN)]
    pub half_extents: Vec3,
    /// Capsule half-height, excluding the hemispherical caps.
    #[reflect(shown_when = HALF_HEIGHT_WHEN)]
    pub half_height: f32,
    /// The shape's centre, in the entity's local space.
    ///
    /// Moves the geometry inside the body without moving the body. A
    /// model whose pivot is not at its centre of volume needs this: a
    /// character pivoted at the feet wants its capsule half a body up, and
    /// a door pivoted on the hinge wants its box beside it rather than
    /// around it.
    pub center: Vec3,
}

impl Default for Collider {
    fn default() -> Self {
        Self {
            shape: SHAPE_SPHERE,
            radius: 0.5,
            half_extents: Vec3::splat(0.5),
            half_height: 0.5,
            center: Vec3::ZERO,
        }
    }
}

impl Component for Collider {}

impl Collider {
    /// Resolves the flat fields to the geometry the backend takes.
    ///
    /// Degenerate values are clamped rather than rejected: a collider
    /// mid-edit in the Inspector passes through zero on the way to the
    /// value the user is typing, and a zero-radius shape makes the
    /// solver produce NaNs that outlive the typo.
    pub fn collision_shape(&self) -> CollisionShape {
        const MIN: f32 = 1e-4;
        match self.shape {
            SHAPE_CUBOID => CollisionShape::Cuboid {
                half_extents: self.half_extents.max(Vec3::splat(MIN)),
            },
            SHAPE_CAPSULE => CollisionShape::Capsule {
                radius: self.radius.max(MIN),
                half_height: self.half_height.max(MIN),
            },
            _ => CollisionShape::Sphere {
                radius: self.radius.max(MIN),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
