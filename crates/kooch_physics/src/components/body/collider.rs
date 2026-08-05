//! [`Collider`] — the shape an entity presents to the solver, and how
//! that surface behaves on contact.
//!
//! Same discriminant rule as [`PhysicsBody`](super::PhysicsBody): `shape` is
//! a `u32` with a choice set, because reflection cannot express an enum.

use glam::Vec3;

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;
use kooch_ecs::reflect::{FieldChoice, FieldCondition};

use crate::backend::{
    ColliderInteraction, CollisionShape, CombineRule, InteractionMask, SurfaceMaterial,
};

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
    /// Resistance to sliding. 0 is frictionless; 1 is about rubber on dry
    /// tarmac. Above 1 is legal and useful for gameplay.
    pub friction: f32,
    /// How this collider's friction combines with the other one's. One of
    /// the `COMBINE_*` constants.
    ///
    /// **The pushier claim wins.** Rapier resolves a pair by taking the
    /// higher of the two discriminants, so a collider on Average against
    /// one on Max gets Max. A rule is less "how my surface behaves" than
    /// "how I insist on being combined".
    #[reflect(choices = COMBINE_CHOICES)]
    pub friction_rule: u32,
    /// Bounce. 0 absorbs the impact; 1 returns it, so a ball comes back to
    /// roughly the height it fell from.
    pub restitution: f32,
    /// How this collider's bounce combines with the other one's. Same
    /// max-wins resolution as `friction_rule`.
    #[reflect(choices = COMBINE_CHOICES)]
    pub restitution_rule: u32,
    /// Report overlap and never push — a trigger volume.
    ///
    /// A sensor is not a collider that gets ignored: rapier computes no
    /// contact manifold for it at all, so its events carry no contact
    /// information. Checkpoints, damage zones, detection ranges.
    pub sensor: bool,
    /// Raise an event when this collider starts or stops touching
    /// something.
    ///
    /// Off by default, and that is the design rather than an oversight:
    /// events are opt-in per collider in rapier, so a scene pays only for
    /// what it listens to.
    pub collision_events: bool,
    /// Raise an event when contact force exceeds
    /// `contact_force_threshold`.
    ///
    /// This is what tells "brushed the wall" from "hit it hard enough to
    /// take damage" without inspecting contacts every frame.
    pub contact_force_events: bool,
    /// The force, in newtons, above which a contact is worth reporting.
    #[reflect(shown_when = CONTACT_FORCE_WHEN)]
    pub contact_force_threshold: f32,
    /// Which groups this collider belongs to.
    ///
    /// A pair is considered only when each side's memberships intersect the
    /// other's filter — **both** directions, so being in a group the other
    /// side looks for is not enough on its own.
    #[reflect(bits = GROUP_BITS)]
    pub collision_memberships: u32,
    /// Which groups this collider will collide with.
    #[reflect(bits = GROUP_BITS)]
    pub collision_filter: u32,
    /// Which groups this collider is *solved* against, out of those it
    /// collides with.
    ///
    /// The pair of masks is the point: a projectile that should detect a
    /// wall without being stopped by it shares the wall's collision groups
    /// and not its solver groups.
    #[reflect(bits = GROUP_BITS)]
    pub solver_memberships: u32,
    /// Which groups this collider will be pushed by.
    #[reflect(bits = GROUP_BITS)]
    pub solver_filter: u32,
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
            friction: 0.5,
            friction_rule: COMBINE_AVERAGE,
            restitution: 0.0,
            restitution_rule: COMBINE_AVERAGE,
            sensor: false,
            collision_events: false,
            contact_force_events: false,
            contact_force_threshold: 0.0,
            collision_memberships: u32::MAX,
            collision_filter: u32::MAX,
            solver_memberships: u32::MAX,
            solver_filter: u32::MAX,
            center: Vec3::ZERO,
        }
    }
}

impl Component for Collider {}

/// The mean of the two coefficients. Rapier's default.
pub const COMBINE_AVERAGE: u32 = 0;
/// The smaller value — the slipperier surface wins.
pub const COMBINE_MIN: u32 = 1;
/// The product — both surfaces have to be high.
pub const COMBINE_MULTIPLY: u32 = 2;
/// The larger value — the stickier surface wins.
pub const COMBINE_MAX: u32 = 3;
/// The sum, clamped.
pub const COMBINE_CLAMPED_SUM: u32 = 4;

/// Labels for the combine-rule dropdowns.
pub static COMBINE_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "Average",
        value: COMBINE_AVERAGE as i64,
    },
    FieldChoice {
        label: "Min (slipperier wins)",
        value: COMBINE_MIN as i64,
    },
    FieldChoice {
        label: "Multiply",
        value: COMBINE_MULTIPLY as i64,
    },
    FieldChoice {
        label: "Max (stickier wins)",
        value: COMBINE_MAX as i64,
    },
    FieldChoice {
        label: "Clamped sum",
        value: COMBINE_CLAMPED_SUM as i64,
    },
];

/// The collision groups, named.
///
/// Sixteen of rapier's thirty-two bits, named generically because the
/// engine does not know what a project's layers mean. A game renames them
/// by shipping its own labels; what matters here is that the Inspector
/// shows *boxes* rather than a number, because a filtering mistake written
/// as an integer fails silently — two things pass through each other and
/// nothing says why.
///
/// The remaining sixteen are deliberately unnamed rather than absent: the
/// widget preserves bits it does not know about, so a project using the
/// high half by hand keeps it across an edit.
pub static GROUP_BITS: &[FieldChoice] = &[
    FieldChoice {
        label: "Group 1",
        value: 1 << 0,
    },
    FieldChoice {
        label: "Group 2",
        value: 1 << 1,
    },
    FieldChoice {
        label: "Group 3",
        value: 1 << 2,
    },
    FieldChoice {
        label: "Group 4",
        value: 1 << 3,
    },
    FieldChoice {
        label: "Group 5",
        value: 1 << 4,
    },
    FieldChoice {
        label: "Group 6",
        value: 1 << 5,
    },
    FieldChoice {
        label: "Group 7",
        value: 1 << 6,
    },
    FieldChoice {
        label: "Group 8",
        value: 1 << 7,
    },
    FieldChoice {
        label: "Group 9",
        value: 1 << 8,
    },
    FieldChoice {
        label: "Group 10",
        value: 1 << 9,
    },
    FieldChoice {
        label: "Group 11",
        value: 1 << 10,
    },
    FieldChoice {
        label: "Group 12",
        value: 1 << 11,
    },
    FieldChoice {
        label: "Group 13",
        value: 1 << 12,
    },
    FieldChoice {
        label: "Group 14",
        value: 1 << 13,
    },
    FieldChoice {
        label: "Group 15",
        value: 1 << 14,
    },
    FieldChoice {
        label: "Group 16",
        value: 1 << 15,
    },
];

/// Which state reads `contact_force_threshold`: only a collider that asked
/// for force events.
pub static CONTACT_FORCE_WHEN: FieldCondition = FieldCondition {
    field: "contact_force_events",
    values: &[1],
};

/// The backend rule for a discriminant, defaulting to the average for one
/// outside the known set — a scene from a newer editor stays loadable.
fn combine_rule(discriminant: u32) -> CombineRule {
    match discriminant {
        COMBINE_MIN => CombineRule::Min,
        COMBINE_MULTIPLY => CombineRule::Multiply,
        COMBINE_MAX => CombineRule::Max,
        COMBINE_CLAMPED_SUM => CombineRule::ClampedSum,
        _ => CombineRule::Average,
    }
}

impl Collider {
    /// The surface this collider presents on contact.
    pub fn material(&self) -> SurfaceMaterial {
        SurfaceMaterial {
            friction: self.friction,
            friction_rule: combine_rule(self.friction_rule),
            restitution: self.restitution,
            restitution_rule: combine_rule(self.restitution_rule),
        }
        .sanitised()
    }

    /// How this collider participates: what it notices and what it
    /// reports.
    pub fn interaction(&self) -> ColliderInteraction {
        ColliderInteraction {
            collision_groups: InteractionMask {
                memberships: self.collision_memberships,
                filter: self.collision_filter,
            },
            solver_groups: InteractionMask {
                memberships: self.solver_memberships,
                filter: self.solver_filter,
            },
            sensor: self.sensor,
            collision_events: self.collision_events,
            contact_force_events: self.contact_force_events,
            contact_force_threshold: self.contact_force_threshold.max(0.0),
        }
    }

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
