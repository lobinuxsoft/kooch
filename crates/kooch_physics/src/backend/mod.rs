//! The physics trait and descriptors. Game code uses [`PhysicsBackend`], implemented by
//! [`crate::RapierBackend`]; the API is glam only.

mod body;
mod debug;
mod events;
mod interaction;
mod joint;
mod material;
mod mesh_cache;
mod mesh_key;
mod query;
mod shape;

pub use body::{BodyDesc, BodyHandle, BodyKind, ColliderHandle, RayHit};
pub use debug::{DebugCategories, DebugLine};
pub use events::{CollisionEvent, ContactForceEvent};
pub use interaction::{ColliderInteraction, InteractionMask};
pub use joint::{BrokenJoint, JointDesc, JointHandle, JointKind, JointMotor, MotorModel};
pub use material::{CombineRule, Damping, SurfaceMaterial};
pub use mesh_cache::{ColliderMesh, ColliderMeshCache};
pub use mesh_key::MeshKey;
pub use query::{PointHit, QueryFilter, ShapeAt, ShapeHit};
pub use shape::{CollisionShape, ConvexPart, MIN_EXTENT};

use glam::{Quat, Vec3};

/// Engine-facing physics interface, stored boxed as a resource with no enum dispatch. Per frame:
/// sync `add/remove_body`, push kinematic poses, `step(dt)`, pull dynamic poses.
pub trait PhysicsBackend: Send + Sync + 'static {
    /// Advances the simulation by `dt` seconds.
    fn step(&mut self, dt: f32);

    /// The uniform acceleration applied to every dynamic body.
    fn gravity(&self) -> Vec3;

    /// Uniform acceleration on every dynamic body — on the trait so gravity fields can turn it off,
    /// or a planet's pull plus world down gives a diagonal.
    fn set_gravity(&mut self, gravity: Vec3);

    /// Inserts a body, returns its handle. Handles are stable across
    /// simulation steps until [`remove_body`](Self::remove_body) is called.
    fn add_body(&mut self, desc: BodyDesc) -> BodyHandle;

    /// Removes a body and its colliders. Subsequent calls with `handle`
    /// return `None` from getters and silently no-op for setters.
    fn remove_body(&mut self, handle: BodyHandle);

    /// Adds a shape to a body: a child `Collider` without `PhysicsBody` joins the nearest ancestor
    /// (compound collider); two simulating bodies want [`add_joint`](Self::add_joint). `material`
    /// is the shape's own; `None` if stale.
    fn attach_collider(
        &mut self,
        body: BodyHandle,
        shape: CollisionShape,
        offset: Vec3,
        rotation: Quat,
        material: SurfaceMaterial,
        interaction: ColliderInteraction,
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

    /// What the body actually weighs in kg, `None` if stale — the descriptor is the request, this
    /// is what was built (#618).
    fn mass(&self, handle: BodyHandle) -> Option<f32>;

    /// Whether the body sleeps, `None` if stale. Ask before applying per-step forces: waking
    /// resting bodies keeps the scene simulating forever.
    fn is_sleeping(&self, handle: BodyHandle) -> Option<bool>;

    /// Centre of mass in body space, `None` if stale — what surprises authors of compound bodies,
    /// and what a debug view draws (#563).
    fn center_of_mass(&self, handle: BodyHandle) -> Option<Vec3>;

    /// Linear velocity in world space. `None` for stale handles or
    /// non-dynamic bodies.
    fn linear_velocity(&self, handle: BodyHandle) -> Option<Vec3>;

    /// Sets linear velocity for dynamic bodies. No-op otherwise.
    fn set_linear_velocity(&mut self, handle: BodyHandle, velocity: Vec3);

    /// Angular velocity in rad/s per world axis, `None` if stale — without it "is angular damping
    /// working" cannot be asked.
    fn angular_velocity(&self, handle: BodyHandle) -> Option<Vec3>;

    /// Sets angular velocity for dynamic bodies. No-op otherwise.
    fn set_angular_velocity(&mut self, handle: BodyHandle, velocity: Vec3);

    /// Instantaneous momentum change — impulses, since rapier's forces persist and accumulate.
    /// **`wake: false` for per-step pushes**, or bodies never sleep. No-op for stale or non-dynamic
    /// bodies.
    fn apply_impulse(&mut self, handle: BodyHandle, impulse: Vec3, wake: bool);

    /// The angular twin of [`apply_impulse`](Self::apply_impulse), same `wake` rules. A ball driven
    /// by linear impulses skids; a torque spins it and friction turns spin into rolling.
    /// Axis × magnitude in N·m·s, world space; no-op for stale or non-dynamic bodies.
    fn apply_torque_impulse(&mut self, handle: BodyHandle, torque: Vec3, wake: bool);

    /// Constrains two bodies; `None` for a stale handle or a joint the backend cannot build (an
    /// articulated loop) — a refusal to report.
    fn add_joint(&mut self, desc: JointDesc) -> Option<JointHandle>;

    /// Removes a joint. Both bodies survive, unconstrained. Idempotent for
    /// a stale handle.
    fn remove_joint(&mut self, handle: JointHandle);

    /// Number of live joints, impulse and articulated together.
    fn joint_count(&self) -> usize;

    /// Impulse holding the joint on the last step, `None` if stale — what
    /// [`JointDesc::break_impulse`] is compared against.
    fn joint_impulse(&self, handle: JointHandle) -> Option<f32>;

    /// Drains the last [`step`](Self::step)'s collisions, so each is seen once and unread ones do
    /// not pile up. Only colliders with [`ColliderInteraction::collision_events`] produce any.
    fn take_collision_events(&mut self) -> Vec<CollisionEvent> {
        Vec::new()
    }

    /// Drains the contact-force events the last step reported.
    fn take_contact_force_events(&mut self) -> Vec<ContactForceEvent> {
        Vec::new()
    }

    /// Drains joints broken in the last [`step`](Self::step) — each seen once, none accumulating.
    fn take_broken_joints(&mut self) -> Vec<BrokenJoint>;

    /// Casts a ray and returns the closest hit, if any. `dir` is expected
    /// to be normalized; `max_t` is the parametric cutoff.
    fn query_ray(&self, origin: Vec3, dir: Vec3, max_t: f32, filter: QueryFilter)
    -> Option<RayHit>;

    /// Every hit along a ray, to a callback (a `Vec` per shot is an allocation per shot); `false`
    /// stops. **Unordered** — the tree decides; sort what you keep.
    fn query_ray_all(
        &self,
        origin: Vec3,
        dir: Vec3,
        max_t: f32,
        filter: QueryFilter,
        out: &mut dyn FnMut(RayHit) -> bool,
    );

    /// Sweeps a shape along `dir`, returning the first hit: a zero-width ray slips through gaps a
    /// body cannot and misses thin walls. `dir` need not be normalised; `max_t` is in its lengths.
    fn query_sweep(
        &self,
        shape: ShapeAt<'_>,
        dir: Vec3,
        max_t: f32,
        filter: QueryFilter,
    ) -> Option<ShapeHit>;

    /// Nearest point on the nearest body within `max_distance`, and whether the point is inside it.
    fn query_point(&self, point: Vec3, max_distance: f32, filter: QueryFilter) -> Option<PointHit>;

    /// Every body a shape overlaps where it stands — explosions, selection boxes. Callback as
    /// [`query_ray_all`](Self::query_ray_all); `false` stops.
    fn query_overlaps(
        &self,
        shape: ShapeAt<'_>,
        filter: QueryFilter,
        out: &mut dyn FnMut(BodyHandle) -> bool,
    );

    /// Appends segments describing the solver's state ([`DebugLine`]) into a reused buffer.
    /// Defaults to nothing: a backend that cannot introspect must not invent a plausible picture.
    fn debug_lines(&self, _categories: DebugCategories, _out: &mut Vec<DebugLine>) {}
}
