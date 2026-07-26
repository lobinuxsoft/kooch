//! Public trait + descriptor types for the physics subsystem.
//!
//! Game code consumes [`PhysicsBackend`]. Concrete backends
//! ([`crate::RapierBackend`] today, `WgrapierBackend` when GPU lands)
//! implement it. All public types use glam — no nalgebra in the API
//! surface, even when the backend uses it internally.

use glam::{Quat, Vec3};

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

/// Collision primitive attached to a body. PR-1 covers the three shapes
/// every game ships; convex hulls + trimesh + heightfield arrive with
/// #137 (CollisionShape ECS component) and the asset pipeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CollisionShape {
    /// Solid ball, parametrised by `radius`.
    Sphere { radius: f32 },
    /// Axis-aligned box (in local space). `half_extents` are half the
    /// total side length on each axis.
    Cuboid { half_extents: Vec3 },
    /// Capsule along the local Y axis. `half_height` excludes the
    /// hemispherical caps; total length = `2 * (half_height + radius)`.
    Capsule { radius: f32, half_height: f32 },
}

/// Construction descriptor handed to [`PhysicsBackend::add_body`].
#[derive(Debug, Clone, Copy)]
pub struct BodyDesc {
    pub kind: BodyKind,
    pub shape: CollisionShape,
    /// Mass in kg. Ignored for [`BodyKind::Static`] /
    /// [`BodyKind::Kinematic`]; used for dynamic inertia tensor.
    pub mass: f32,
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

/// Result of a successful [`PhysicsBackend::query_ray`] call.
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

/// Engine-facing physics interface.
///
/// Backends are stored as a [`Resource`](ome_core::resource::Resources)
/// boxed behind this trait. Systems call methods directly; no enum
/// dispatch on backend kind in hot paths.
///
/// # Lifecycle
///
/// 1. Engine inserts a backend at startup
///    (`resources.insert(Box::new(RapierBackend::new()) as Box<dyn PhysicsBackend>)`).
/// 2. Per frame: ECS `add/remove_body` syncs lifetime, `set_transform`
///    pushes kinematic poses, `step(dt)` advances simulation,
///    `get_transform` pulls dynamic poses back to ECS.
/// 3. On shutdown: dropping the resource releases everything.
pub trait PhysicsBackend: Send + Sync + 'static {
    /// Advances the simulation by `dt` seconds.
    fn step(&mut self, dt: f32);

    /// Inserts a body, returns its handle. Handles are stable across
    /// simulation steps until [`remove_body`](Self::remove_body) is called.
    fn add_body(&mut self, desc: BodyDesc) -> BodyHandle;

    /// Removes a body and its colliders. Subsequent calls with `handle`
    /// return `None` from getters and silently no-op for setters.
    fn remove_body(&mut self, handle: BodyHandle);

    /// Adds another collision shape to an existing body.
    ///
    /// This is how a hierarchy becomes physics: a child entity carrying a
    /// `Collider` but no `RigidBody` of its own contributes its shape to
    /// the nearest ancestor that has one. The result is **one** body with
    /// several shapes, which is what Unity calls a compound collider and
    /// Unreal calls welding.
    ///
    /// The alternative — one body per collider, held together by the
    /// transform hierarchy — is the thing no engine supports, because the
    /// solver and the hierarchy would both own the pose.
    ///
    /// `offset` and `rotation` place the shape in the body's local space.
    /// Returns `None` for a stale body handle.
    fn attach_collider(
        &mut self,
        body: BodyHandle,
        shape: CollisionShape,
        offset: Vec3,
        rotation: Quat,
    ) -> Option<ColliderHandle>;

    /// Removes one attached shape. The body and its other shapes survive.
    fn detach_collider(&mut self, handle: ColliderHandle);

    /// Number of shapes attached to a body, including the one it was
    /// created with. `None` for a stale handle.
    fn collider_count(&self, body: BodyHandle) -> Option<usize>;

    /// Returns `true` when the handle is live.
    fn contains(&self, handle: BodyHandle) -> bool;

    /// Number of live bodies.
    fn body_count(&self) -> usize;

    /// Reads the body's current world-space transform. `None` for stale
    /// handles.
    fn get_transform(&self, handle: BodyHandle) -> Option<(Vec3, Quat)>;

    /// Sets the body's world-space transform. For [`BodyKind::Dynamic`]
    /// this teleports — solver does NOT integrate impulses across the
    /// move. For kinematic bodies this is the standard way to drive them.
    fn set_transform(&mut self, handle: BodyHandle, position: Vec3, rotation: Quat);

    /// Linear velocity in world space. `None` for stale handles or
    /// non-dynamic bodies.
    fn linear_velocity(&self, handle: BodyHandle) -> Option<Vec3>;

    /// Sets linear velocity for dynamic bodies. No-op otherwise.
    fn set_linear_velocity(&mut self, handle: BodyHandle, velocity: Vec3);

    /// Casts a ray and returns the closest hit, if any. `dir` is expected
    /// to be normalized; `max_t` is the parametric cutoff.
    fn query_ray(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Option<RayHit>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_desc_dynamic_default_at_origin() {
        let desc = BodyDesc::dynamic(CollisionShape::Sphere { radius: 1.0 }, 5.0);
        assert_eq!(desc.kind, BodyKind::Dynamic);
        assert_eq!(desc.position, Vec3::ZERO);
        assert_eq!(desc.rotation, Quat::IDENTITY);
        assert_eq!(desc.mass, 5.0);
    }

    #[test]
    fn body_desc_static_uses_position() {
        let pos = Vec3::new(1.0, 2.0, 3.0);
        let desc = BodyDesc::static_at(
            CollisionShape::Cuboid {
                half_extents: Vec3::splat(0.5),
            },
            pos,
        );
        assert_eq!(desc.kind, BodyKind::Static);
        assert_eq!(desc.position, pos);
        assert_eq!(desc.mass, 0.0);
    }

    #[test]
    fn collision_shapes_cover_three_primitives() {
        let _s = CollisionShape::Sphere { radius: 1.0 };
        let _c = CollisionShape::Cuboid {
            half_extents: Vec3::ONE,
        };
        let _cap = CollisionShape::Capsule {
            radius: 0.5,
            half_height: 1.0,
        };
    }
}
