//! ECS components describing an entity's physical intent.
//!
//! These say *what* an entity is physically — a dynamic body, a sphere
//! of radius 0.5, a hinge between two of them — never *how* it is
//! simulated. Nothing here mentions the backend, so swapping the solver
//! (wgrapier, when GPU rigid bodies land) is a change inside
//! [`crate::rapier_backend`], not a rewrite of every scene ever authored.
//!
//! # Why flat fields instead of enums
//!
//! Reflection has no enum representation — [`ReflectValue`] covers
//! scalars, vectors and asset references — so a variant is a `u32`
//! discriminant with labelled `choices`, and the parameters of every
//! variant sit side by side. The alternative was blocking physics on a
//! reflection feature; the shape of the data is the same either way, and
//! each component resolves it back to a typed backend enum at the seam —
//! [`Collider::collision_shape`], [`Joint::joint_kind`].
//!
//! Only the fields belonging to the selected variant are *shown*, via
//! `FieldCondition`. Hiding is display only: every field is still stored,
//! still serialised, still round-trips through a scene, so switching
//! variant back and forth never loses the other one's parameters.
//!
//! [`ReflectValue`]: ome_ecs::reflect::ReflectValue

mod body;
mod joint;

pub use body::{
    CENTER_OF_MASS_WHEN, COMBINE_AVERAGE, COMBINE_CHOICES, COMBINE_CLAMPED_SUM, COMBINE_MAX,
    COMBINE_MIN, COMBINE_MULTIPLY, CONTACT_FORCE_WHEN, Collider, HALF_EXTENTS_WHEN,
    HALF_HEIGHT_WHEN, KIND_CHOICES, KIND_DYNAMIC, KIND_KINEMATIC, KIND_STATIC, RADIUS_WHEN,
    RigidBody, SHAPE_CAPSULE, SHAPE_CHOICES, SHAPE_CUBOID, SHAPE_SPHERE,
};
pub use joint::{
    AXIS_WHEN, BREAKABLE_WHEN, FREE_AXIS_WHEN, JOINT_FIXED, JOINT_GENERIC, JOINT_KIND_CHOICES,
    JOINT_PIN_SLOT, JOINT_PRISMATIC, JOINT_REVOLUTE, JOINT_ROPE, JOINT_SPHERICAL, JOINT_SPRING,
    Joint, LOCKED_AXES_WHEN, MAX_LENGTH_WHEN, MOTOR_ACCELERATION, MOTOR_FORCE, MOTOR_MODEL_CHOICES,
    SPRING_WHEN,
};
