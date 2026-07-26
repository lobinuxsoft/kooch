//! Public trait + descriptor types for the physics subsystem.
//!
//! Game code consumes [`PhysicsBackend`]. Concrete backends
//! ([`crate::RapierBackend`] today, `WgrapierBackend` when GPU lands)
//! implement it. All public types use glam — no nalgebra in the API
//! surface, even when the backend uses it internally.

mod body;
mod joint;

pub use body::{BodyDesc, BodyHandle, BodyKind, ColliderHandle, CollisionShape, RayHit};
pub use joint::{BrokenJoint, JointDesc, JointHandle, JointKind, JointMotor, MotorModel};

use glam::{Quat, Vec3};

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
    /// solver and the hierarchy would both own the pose. Two bodies that
    /// both simulate want [`add_joint`](Self::add_joint) instead.
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

    /// What the body actually weighs, in kg. `None` for a stale handle.
    ///
    /// Worth asking rather than assuming: the descriptor says what was
    /// requested, and this says what the solver built. #618 was filed
    /// because those two had silently drifted apart.
    fn mass(&self, handle: BodyHandle) -> Option<f32>;

    /// The body's centre of mass, in body-local space. `None` for a stale
    /// handle.
    ///
    /// The thing a compound body surprises authors with, and what a
    /// physics debug view has to draw (#563).
    fn center_of_mass(&self, handle: BodyHandle) -> Option<Vec3>;

    /// Linear velocity in world space. `None` for stale handles or
    /// non-dynamic bodies.
    fn linear_velocity(&self, handle: BodyHandle) -> Option<Vec3>;

    /// Sets linear velocity for dynamic bodies. No-op otherwise.
    fn set_linear_velocity(&mut self, handle: BodyHandle, velocity: Vec3);

    /// Constrains two bodies to each other.
    ///
    /// Returns `None` when either body handle is stale, or when the
    /// descriptor asks for something the backend cannot build — an
    /// articulated joint closing a loop, most usefully. A `None` is a
    /// refusal the caller can report, not a silent no-op.
    fn add_joint(&mut self, desc: JointDesc) -> Option<JointHandle>;

    /// Removes a joint. Both bodies survive, unconstrained. Idempotent for
    /// a stale handle.
    fn remove_joint(&mut self, handle: JointHandle);

    /// Number of live joints, impulse and articulated together.
    fn joint_count(&self) -> usize;

    /// Magnitude of the impulse the solver applied to hold a joint together
    /// on the last step. `None` for a stale handle.
    ///
    /// This is the load on the constraint, and it is what
    /// [`JointDesc::break_impulse`] is compared against.
    fn joint_impulse(&self, handle: JointHandle) -> Option<f32>;

    /// Drains the joints that broke during the last [`step`](Self::step).
    ///
    /// Draining rather than peeking, so a caller that reads it every frame
    /// sees each break exactly once and a caller that never reads it does
    /// not accumulate forever.
    fn take_broken_joints(&mut self) -> Vec<BrokenJoint>;

    /// Casts a ray and returns the closest hit, if any. `dir` is expected
    /// to be normalized; `max_t` is the parametric cutoff.
    fn query_ray(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Option<RayHit>;
}
