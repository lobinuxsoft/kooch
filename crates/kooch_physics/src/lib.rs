//! Physics: [`PhysicsBackend`], the trait game code uses, and [`RapierBackend`] on [`rapier3d`] — a
//! GPU solver (#1117) replaces the backend, not gameplay. [`ColliderMeshCache`] is filled from
//! outside, so physics never sees assets.

pub mod backend;
pub mod components;
pub mod plugin;
pub mod rapier_backend;

pub use backend::{
    BodyDesc, BodyHandle, BodyKind, BrokenJoint, ColliderHandle, ColliderMesh, ColliderMeshCache,
    CollisionShape, ConvexPart, JointDesc, JointHandle, JointKind, JointMotor, MotorModel,
    PhysicsBackend, PointHit, QueryFilter, RayHit, ShapeAt, ShapeHit,
};
pub use components::{Collider, Joint, PhysicsBody, ShapeSpec};
pub use plugin::{JointRegistry, PhysicsComponentsPlugin, PhysicsPlugin, PhysicsWorld, SolverBody};
pub use rapier_backend::{RapierBackend, decompose, hull_of};
