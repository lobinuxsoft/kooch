//! [`RigidBody`] and [`Collider`] — what an entity is physically.
//!
//! See the [module docs](super) for why a variant is a `u32` discriminant
//! rather than an enum.

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

/// Which state reads `center_of_mass`: only an explicit override.
pub static CENTER_OF_MASS_WHEN: FieldCondition = FieldCondition {
    field: "center_of_mass_enabled",
    values: &[1],
};

/// Marks an entity as participating in the physics simulation.
///
/// Pairs with a [`Collider`]; an entity carrying only this gets a unit
/// sphere, so a half-authored entity falls rather than panics.
///
/// # Where the mass comes from
///
/// From [`mass`](Self::mass), and nowhere else. Colliders contribute
/// collision and no mass at all.
///
/// This is a deliberate departure from Unity and Unreal, which derive a
/// body's mass properties from every attached shape's volume and density.
/// That is physically correct and it is what surprised the author who
/// filed #618: adding a second collider to a body made it heavier and
/// slower to turn, with nothing in the Inspector saying so. Worse, the
/// `mass` field did not mean kilograms — rapier's `additional_mass` is
/// *added* to the shape-derived mass, so a 1 kg body with a two-metre
/// sphere weighed thirty-four.
///
/// A field labelled kilograms has to mean kilograms. So the shapes are
/// massless, `mass` is the whole mass, and the body's inertia is derived
/// from the entity's *own* collider scaled to that mass — which is also
/// why a compound body's centre of mass stays where the author expects
/// instead of drifting towards the children.
///
/// What that gives up is "a bigger rock is automatically heavier".
/// [`density`](Self::density) buys it back on demand: the Inspector's
/// **Calculate mass** button multiplies it by the colliders' volume and
/// writes the result here, once, where you can see and edit it.
///
/// # Default
///
/// A dynamic body of 1 kg at the density of water.
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Physics")]
pub struct RigidBody {
    /// How the solver treats this body. One of the `KIND_*` constants.
    #[reflect(choices = KIND_CHOICES)]
    pub kind: u32,
    /// Mass in kilograms — the body's whole mass. Ignored by static and
    /// kinematic bodies.
    pub mass: f32,
    /// Kilograms per cubic metre, for the Inspector's **Calculate mass**
    /// button.
    ///
    /// **The simulation never reads this.** It is authoring input: the
    /// button multiplies it by the volume of this entity's colliders and
    /// writes the product into [`mass`](Self::mass). Kept as a field
    /// rather than asked for in a dialog so the number that produced a
    /// mass is visible next to it — 1000 is water, ~2700 aluminium, ~7850
    /// steel, ~600 dry pine.
    pub density: f32,
    /// Put the centre of mass somewhere other than the collider's centre.
    pub center_of_mass_enabled: bool,
    /// The centre of mass, in the entity's local space.
    ///
    /// What Unity calls `centerOfMass` and Unreal calls `COMOffset`. A
    /// vehicle wants its centre of mass low or it rolls in every corner,
    /// and no arrangement of collision shapes expresses that as directly
    /// as saying where it is.
    #[reflect(shown_when = CENTER_OF_MASS_WHEN)]
    pub center_of_mass: Vec3,
}

impl Default for RigidBody {
    fn default() -> Self {
        Self {
            kind: KIND_DYNAMIC,
            mass: 1.0,
            density: 1000.0,
            center_of_mass_enabled: false,
            center_of_mass: Vec3::ZERO,
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

    /// The authored centre of mass, or `None` to use the collider's.
    pub fn explicit_center_of_mass(&self) -> Option<Vec3> {
        self.center_of_mass_enabled.then_some(self.center_of_mass)
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
