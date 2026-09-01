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
//! The one thing a flat field cannot hold is a mesh, so the shapes built
//! from one name a [`Guid`](kooch_core::Guid) and something outside
//! physics resolves it — see
//! [`ColliderMeshCache`](crate::backend::ColliderMeshCache).
//!
//! Only the fields belonging to the selected variant are *shown*, via
//! `FieldCondition`. Hiding is display only: every field is still stored,
//! still serialised, still round-trips through a scene, so switching
//! variant back and forth never loses the other one's parameters.
//!
//! [`ReflectValue`]: kooch_ecs::reflect::ReflectValue

mod body;
mod joint;

pub use body::{
    BORDER_RADIUS_WHEN, CENTER_OF_MASS_WHEN, COMBINE_AVERAGE, COMBINE_CHOICES, COMBINE_CLAMPED_SUM,
    COMBINE_MAX, COMBINE_MIN, COMBINE_MULTIPLY, CONTACT_FORCE_WHEN, Collider, ENDPOINTS_WHEN,
    GROUP_BITS, HALF_EXTENTS_WHEN, HALF_HEIGHT_WHEN, KIND_CHOICES, KIND_DYNAMIC, KIND_KINEMATIC,
    KIND_STATIC, MESH_DERIVED, MESH_WHEN, NORMAL_WHEN, POINT_C_WHEN, PhysicsBody, RADIUS_WHEN,
    SHAPE_CAPSULE, SHAPE_CHOICES, SHAPE_CONE, SHAPE_CONVEX_DECOMPOSITION, SHAPE_CONVEX_HULL,
    SHAPE_CUBOID, SHAPE_CYLINDER, SHAPE_HALF_SPACE, SHAPE_POLYLINE, SHAPE_ROUND_CYLINDER,
    SHAPE_SEGMENT, SHAPE_SPHERE, SHAPE_TRIANGLE, SHAPE_TRIMESH, SHAPE_VOXELIZED_MESH, SHAPE_VOXELS,
    ShapeSpec, VOXEL_SIZE_WHEN, VOXEL_SOLID_WHEN, is_mesh_derived,
};
pub use joint::{
    AXIS_WHEN, BREAKABLE_WHEN, FREE_AXIS_WHEN, JOINT_FIXED, JOINT_GENERIC, JOINT_KIND_CHOICES,
    JOINT_PIN_SLOT, JOINT_PRISMATIC, JOINT_REVOLUTE, JOINT_ROPE, JOINT_SPHERICAL, JOINT_SPRING,
    Joint, LOCKED_AXES_WHEN, MAX_LENGTH_WHEN, MOTOR_ACCELERATION, MOTOR_FORCE, MOTOR_MODEL_CHOICES,
    SPRING_WHEN,
};
