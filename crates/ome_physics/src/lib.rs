//! ome_physics — physics subsystem.
//!
//! Provides the [`PhysicsBackend`] trait — the engine's physics interface —
//! and a CPU-side implementation backed by [`rapier3d`]. Game code talks to
//! the trait; swapping providers (e.g. wgrapier3d when GPU rigid body
//! ships) replaces the backend without touching gameplay.
//!
//! # Architecture
//!
//! - [`backend`] — public trait + descriptor types (BodyDesc, BodyHandle,
//!   CollisionShape, RayHit). Pure data + glam types, no nalgebra leaks.
//! - [`rapier_backend`] — concrete [`RapierBackend`] implementing the trait
//!   on top of Rapier's CPU pipeline. Owns the `RigidBodySet`,
//!   `ColliderSet`, `IslandManager`, etc.
//!
//! # Pivot note (2026-05-02)
//!
//! Replaced the legacy SDF broadphase (#42 closed as superseded). Physics
//! is now industry-standard rigid body via Rapier. SDF queries against
//! terrain are *separate* — they belong to the voxel storage in
//! `ome_world::voxel` once the dual contouring pipeline (Phase 2.5)
//! starts feeding voxel zones.

pub mod backend;
pub mod components;
pub mod plugin;
pub mod rapier_backend;

pub use backend::{
    BodyDesc, BodyHandle, BodyKind, ColliderHandle, CollisionShape, PhysicsBackend, RayHit,
};
pub use components::{Collider, RigidBody};
pub use plugin::{PhysicsBody, PhysicsComponentsPlugin, PhysicsPlugin, PhysicsWorld};
pub use rapier_backend::RapierBackend;
