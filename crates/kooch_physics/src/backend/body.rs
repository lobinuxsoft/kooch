//! Bodies, the shapes they carry, and the handles that address them.

use glam::{Quat, Vec3};

use super::interaction::ColliderInteraction;
use super::material::{Damping, SurfaceMaterial};
use super::shape::CollisionShape;

/// How the solver treats a body's motion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyKind {
    /// Solver-driven: gravity, forces, collisions push it around.
    Dynamic,
    /// Author-driven: position is set explicitly, but other dynamics
    /// react to it (collisions push *them* off, not the kinematic body).
    Kinematic,
    /// Immovable: never moved by the solver, never moved by user.
    Static,
}

/// Descriptor for [`PhysicsBackend::add_body`](super::PhysicsBackend::add_body). `Clone`, not
/// `Copy`: a mesh-derived [`CollisionShape`] owns its points, and copying a level's trimesh by
/// accident is unaffordable.
#[derive(Debug, Clone)]
pub struct BodyDesc {
    pub kind: BodyKind,
    pub shape: CollisionShape,
    /// The body's **whole** mass in kg, ignored for static and kinematic bodies. Shapes add no
    /// mass, so it means the same however many colliders there are (#618); inertia still comes from
    /// [`shape`](Self::shape), scaled to it.
    pub mass: f32,
    /// Centre of mass in body-local space, `None` for the shape's — a vehicle wants it low or it
    /// rolls in every corner.
    pub center_of_mass: Option<Vec3>,
    /// What the body's own shape does on contact.
    pub material: SurfaceMaterial,
    /// What the body's own shape notices and reports.
    pub interaction: ColliderInteraction,
    /// How quickly the body loses motion with nothing touching it.
    pub damping: Damping,
    /// Multiplier on the world's gravity for this body. 1 is normal, 0 is
    /// weightless, negative rises.
    pub gravity_scale: f32,
    pub position: Vec3,
    pub rotation: Quat,
    /// The shape's centre in body-local space, a plain vector so a GPU backend can honour the same
    /// descriptor (Rapier's `position_wrt_parent`).
    pub shape_offset: Vec3,
}

impl BodyDesc {
    /// Convenience constructor — dynamic body at world origin.
    pub fn dynamic(shape: CollisionShape, mass: f32) -> Self {
        Self {
            kind: BodyKind::Dynamic,
            shape,
            mass,
            center_of_mass: None,
            material: SurfaceMaterial::default(),
            interaction: ColliderInteraction::default(),
            damping: Damping::default(),
            gravity_scale: 1.0,
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            shape_offset: Vec3::ZERO,
        }
    }

    #[cfg(test)]
    /// Convenience constructor — static body at world origin.
    pub fn static_at(shape: CollisionShape, position: Vec3) -> Self {
        Self {
            kind: BodyKind::Static,
            shape,
            mass: 0.0,
            center_of_mass: None,
            material: SurfaceMaterial::default(),
            interaction: ColliderInteraction::default(),
            damping: Damping::default(),
            gravity_scale: 1.0,
            position,
            rotation: Quat::IDENTITY,
            shape_offset: Vec3::ZERO,
        }
    }
}

slotmap::new_key_type! {
    /// Opaque, 16 B handle per body; a stale handle yields `None` through slotmap's generation
    /// counter.
    pub struct BodyHandle;
}

slotmap::new_key_type! {
    /// Handle for one shape on a body — separate from [`BodyHandle`], since removing a child's
    /// collider must not remove the body.
    pub struct ColliderHandle;
}

/// Result of a successful [`query_ray`] call.
///
/// [`query_ray`]: super::PhysicsBackend::query_ray
#[derive(Debug, Clone, Copy)]
pub struct RayHit {
    /// Body the ray hit.
    pub body: BodyHandle,
    /// Parametric distance along the ray (0 = origin, 1 = origin+dir).
    pub t: f32,
    /// World-space hit point.
    pub point: Vec3,
    /// World-space surface normal at the hit.
    pub normal: Vec3,
}

#[cfg(test)]
mod tests;
