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

/// Construction descriptor handed to [`add_body`].
///
/// Cloned rather than copied: [`CollisionShape`] owns its points once a
/// collider is mesh-derived, and a descriptor that copies a level's
/// trimesh by accident is not a descriptor anyone can afford.
///
/// [`add_body`]: super::PhysicsBackend::add_body
#[derive(Debug, Clone)]
pub struct BodyDesc {
    pub kind: BodyKind,
    pub shape: CollisionShape,
    /// The body's **whole** mass in kg. Ignored for [`BodyKind::Static`]
    /// and [`BodyKind::Kinematic`].
    ///
    /// Whole, not additional: shapes contribute collision and no mass, so
    /// this is exactly what the body weighs however many colliders it
    /// carries. A backend that let the shapes add to it would make the
    /// number mean something different for every collider, which is the
    /// bug #618 was filed about.
    ///
    /// The inertia tensor is still derived from [`shape`](Self::shape) —
    /// scaled to this mass — because a body has to resist rotation like
    /// something of its size, and a mass with no geometry behind it has no
    /// tensor at all.
    pub mass: f32,
    /// Where the centre of mass sits in body-local space, or `None` to use
    /// the shape's own centre.
    ///
    /// A vehicle wants its centre of mass low or it rolls in every corner,
    /// and no arrangement of collision shapes says that as directly.
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
    /// The shape's centre relative to the body, in body-local space.
    ///
    /// A plain vector rather than a backend pose type: the trait is the
    /// contract, and a GPU backend later has to be able to honour the same
    /// descriptor. Rapier models this as the collider's
    /// `position_wrt_parent`.
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
    /// Opaque handle issued by the backend for each body it owns.
    /// Cheap to copy (16 B), comparable, hashable. Stale handles
    /// (after `remove_body`) yield `None` from getters thanks to
    /// slotmap's generation counter.
    pub struct BodyHandle;
}

slotmap::new_key_type! {
    /// Opaque handle for one shape attached to a body.
    ///
    /// Separate from [`BodyHandle`] because a body owns several: removing
    /// a child entity's collider must not take the body with it.
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
