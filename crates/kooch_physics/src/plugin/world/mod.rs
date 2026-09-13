//! [`PhysicsWorld`] maps entities to backend bodies by dense `u32` slots on the POD [`SolverBody`]:
//! array walks both ways. It exists because Rapier owns its sets; the trait lets a solver swap
//! without scene changes.

mod queries;

use glam::{Quat, Vec3};

use kooch_ecs::component::Component;
use kooch_ecs::entity::Entity;

use crate::backend::{
    BodyDesc, BodyHandle, ColliderInteraction, ColliderMeshCache, CollisionShape, Damping,
    PhysicsBackend, SurfaceMaterial,
};
use crate::components::{Collider, PhysicsBody, ShapeSpec};

/// An entity's body as a slot into [`PhysicsWorld`]. Unreflected runtime state: never in scenes or
/// [`WorldSnapshot`](kooch_ecs::world_snapshot::WorldSnapshot), so stop drops it and sync rebuilds
/// from the ECS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SolverBody(u32);

impl Component for SolverBody {}

impl SolverBody {
    /// Wraps a slot index.
    pub const fn new(slot: u32) -> Self {
        Self(slot)
    }

    /// The slot this body occupies in [`PhysicsWorld`].
    pub const fn slot(&self) -> u32 {
        self.0
    }
}

/// The authored intent a body was built from, so sync spots mismatches without asking the backend.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodySpec {
    kind: u32,
    mass: f32,
    /// The geometry spec, not the shape — compared every frame, carrying the mesh epoch.
    shape: ShapeSpec,
    /// Shape centre, baked at build, so an Inspector edit must replace the collider.
    center: Vec3,
    /// `Transform` scale folded into dimensions: rapier shapes take no scale, so a scale drag
    /// retires the body.
    scale: Vec3,
    /// Digest of inherited shapes, keeping the spec POD; any child collider change rebuilds the
    /// body.
    attachments: u64,
    /// Surface and damping, baked by rapier at build, so edits rebuild; unlike `density`, the
    /// simulation reads them.
    material: SurfaceMaterial,
    interaction: ColliderInteraction,
    damping: Damping,
    /// In the spec because rapier bakes it into the body at build time.
    gravity_scale: f32,
    /// Authored centre of mass, baked at build. `density` is deliberately absent, so editing it
    /// never drops a body's velocity.
    center_of_mass: Option<Vec3>,
}

impl BodySpec {
    /// Reads the spec off the components; ignoring `scale` made colliders "work at some sizes".
    pub fn new(
        body: &PhysicsBody,
        collider: &Collider,
        entity: kooch_ecs::entity::Entity,
        scale: Vec3,
        meshes: Option<&ColliderMeshCache>,
    ) -> Self {
        Self::with_attachments(body, collider, entity, scale, 0, meshes)
    }

    /// Same, for a body that inherits shapes from its descendants.
    pub fn with_attachments(
        body: &PhysicsBody,
        collider: &Collider,
        entity: kooch_ecs::entity::Entity,
        scale: Vec3,
        attachments: u64,
        meshes: Option<&ColliderMeshCache>,
    ) -> Self {
        Self {
            attachments,
            kind: body.kind,
            mass: body.mass,
            shape: collider.shape_spec(entity, meshes),
            center: collider.center,
            material: collider.material(),
            interaction: collider.interaction(),
            damping: body.damping(),
            gravity_scale: body.gravity_scale,
            center_of_mass: body.explicit_center_of_mass(),
            scale,
        }
    }

    /// The geometry at authored scale; `None` while a mesh waits, retried cheaply thanks to the
    /// epoch.
    pub fn resolve(&self, meshes: Option<&ColliderMeshCache>) -> Option<CollisionShape> {
        Some(self.shape.resolve(meshes)?.scaled(self.scale))
    }

    /// `true` when this body names a mesh nothing has answered for.
    pub fn awaits_mesh(&self, meshes: Option<&ColliderMeshCache>) -> bool {
        self.shape.awaits_mesh(meshes)
    }

    /// The build descriptor at a pose, taking the already-resolved shape.
    pub fn desc(&self, shape: CollisionShape, position: Vec3, rotation: Quat) -> BodyDesc {
        let s = self.scale.abs();
        BodyDesc {
            kind: PhysicsBody {
                kind: self.kind,
                mass: self.mass,
                ..Default::default()
            }
            .body_kind(),
            shape,
            mass: self.mass,
            // Scaled with the body, because it is a point in the entity's
            // local space and the gizmo that scales the shape scales the
            // space the point lives in.
            center_of_mass: self.center_of_mass.map(|center| center * s),
            material: self.material,
            interaction: self.interaction,
            damping: self.damping,
            gravity_scale: self.gravity_scale,
            position,
            rotation,
            // Body-local, not pre-rotated: rapier composes the body pose on `position_wrt_parent`.
            shape_offset: self.center * s,
        }
    }

    /// `true` when the solver — not the author — owns this body's pose.
    pub fn is_dynamic(&self) -> bool {
        matches!(self.kind, crate::components::KIND_DYNAMIC)
    }

    /// `true` when the author drives the pose and the solver reacts.
    pub fn is_kinematic(&self) -> bool {
        matches!(self.kind, crate::components::KIND_KINEMATIC)
    }
}

/// One slot's worth of state. `entity == Entity::INVALID` marks it free.
struct Slot {
    entity: Entity,
    handle: BodyHandle,
    spec: BodySpec,
}

/// The backend plus the body ↔ entity mapping. Slots never compact; freed ones are reused, so
/// indices stay valid.
pub struct PhysicsWorld {
    backend: Box<dyn PhysicsBackend>,
    slots: Vec<Slot>,
    free: Vec<u32>,
    joints: super::joints::JointRegistry,
}

impl PhysicsWorld {
    /// Wraps a backend in an empty world.
    pub fn new(backend: Box<dyn PhysicsBackend>) -> Self {
        Self {
            backend,
            slots: Vec::new(),
            free: Vec::new(),
            joints: Default::default(),
        }
    }

    /// The authored joints and what the solver made of them.
    pub fn joints(&self) -> &super::joints::JointRegistry {
        &self.joints
    }

    /// The joint registry, for the sync pass.
    pub(super) fn joints_mut(&mut self) -> &mut super::joints::JointRegistry {
        &mut self.joints
    }

    /// The backend, for queries.
    pub fn backend(&self) -> &dyn PhysicsBackend {
        self.backend.as_ref()
    }

    /// The backend, for stepping and mutation.
    pub fn backend_mut(&mut self) -> &mut dyn PhysicsBackend {
        self.backend.as_mut()
    }

    /// Number of live bodies.
    pub fn len(&self) -> usize {
        self.slots.len() - self.free.len()
    }

    /// `true` when no body is live.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Highest slot index ever handed out, plus one.
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Creates a body for `entity` and returns the slot addressing it.
    pub fn insert(
        &mut self,
        entity: Entity,
        spec: BodySpec,
        shape: CollisionShape,
        position: Vec3,
        rotation: Quat,
    ) -> u32 {
        let handle = self.backend.add_body(spec.desc(shape, position, rotation));
        let slot = Slot {
            entity,
            handle,
            spec,
        };
        match self.free.pop() {
            Some(index) => {
                self.slots[index as usize] = slot;
                index
            }
            None => {
                self.slots.push(slot);
                (self.slots.len() - 1) as u32
            }
        }
    }

    /// Releases a slot's body. Idempotent for an already-free slot.
    pub fn remove(&mut self, slot: u32) {
        let Some(entry) = self.slots.get_mut(slot as usize) else {
            return;
        };
        if !entry.entity.is_valid() {
            return;
        }
        entry.entity = Entity::INVALID;
        let handle = entry.handle;
        self.backend.remove_body(handle);
        self.free.push(slot);
    }

    /// Drops every body, keeping the backend.
    pub fn clear(&mut self) {
        for slot in 0..self.slots.len() as u32 {
            self.remove(slot);
        }
    }

    /// The entity occupying a slot, or `None` when it is free.
    pub fn entity(&self, slot: u32) -> Option<Entity> {
        self.slots
            .get(slot as usize)
            .filter(|s| s.entity.is_valid())
            .map(|s| s.entity)
    }

    /// The spec a live slot's body was built from.
    pub fn spec(&self, slot: u32) -> Option<BodySpec> {
        self.slots
            .get(slot as usize)
            .filter(|s| s.entity.is_valid())
            .map(|s| s.spec)
    }

    /// The backend handle for a live slot.
    pub fn handle(&self, slot: u32) -> Option<BodyHandle> {
        self.slots
            .get(slot as usize)
            .filter(|s| s.entity.is_valid())
            .map(|s| s.handle)
    }

    /// The entity owning a body — a linear walk, once per opt-in event.
    pub fn entity_of(&self, handle: BodyHandle) -> Option<Entity> {
        self.slots
            .iter()
            .find(|slot| slot.entity.is_valid() && slot.handle == handle)
            .map(|slot| slot.entity)
    }

    /// Walks the live slots as `(slot, entity, spec, handle)`.
    pub fn iter(&self) -> impl Iterator<Item = (u32, Entity, BodySpec, BodyHandle)> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.entity.is_valid())
            .map(|(index, s)| (index as u32, s.entity, s.spec, s.handle))
    }
}

/// Gameplay operations on a [`SolverBody`], hiding slot, handle and backend; the lower-level path
/// stays public. A body that is gone returns `None` or does nothing — ordinary after stop.
impl PhysicsWorld {
    /// Spins a body — the twin of [`apply_impulse`](Self::apply_impulse), what a rolling ball
    /// wants: torque spins it and friction carries it.
    pub fn apply_torque_impulse(&mut self, body: SolverBody, torque: Vec3) {
        let Some(handle) = self.handle(body.slot()) else {
            return;
        };
        self.backend_mut()
            .apply_torque_impulse(handle, torque, true);
    }

    /// Pushes a body in N·s, always waking it — an impulse left asleep did nothing; the
    /// solver-level path is [`backend_mut`](PhysicsWorld::backend_mut).
    pub fn apply_impulse(&mut self, body: SolverBody, impulse: Vec3) {
        let Some(handle) = self.handle(body.slot()) else {
            return;
        };
        self.backend_mut().apply_impulse(handle, impulse, true);
    }

    /// How fast a body is moving, in m/s.
    pub fn linear_velocity(&self, body: SolverBody) -> Option<Vec3> {
        self.backend().linear_velocity(self.handle(body.slot())?)
    }

    /// Sets velocity outright, discarding this step's slope, bounce or collision; prefer
    /// [`apply_impulse`](PhysicsWorld::apply_impulse).
    pub fn set_linear_velocity(&mut self, body: SolverBody, velocity: Vec3) {
        let Some(handle) = self.handle(body.slot()) else {
            return;
        };
        self.backend_mut().set_linear_velocity(handle, velocity);
    }

    /// Turns a body in place for authored orientation (a character facing the steering). Pair with
    /// [`set_angular_velocity`](Self::set_angular_velocity), or leftover spin turns it back.
    pub fn set_rotation(&mut self, body: SolverBody, rotation: Quat) {
        let Some(handle) = self.handle(body.slot()) else {
            return;
        };
        let Some((position, _)) = self.backend().get_transform(handle) else {
            return;
        };
        self.backend_mut().set_transform(handle, position, rotation);
    }

    /// Spin in rad/s, the twin of [`set_linear_velocity`](Self::set_linear_velocity), with the same
    /// warning.
    pub fn set_angular_velocity(&mut self, body: SolverBody, velocity: Vec3) {
        let Some(handle) = self.handle(body.slot()) else {
            return;
        };
        self.backend_mut().set_angular_velocity(handle, velocity);
    }

    /// How fast a body is spinning, in rad/s.
    pub fn angular_velocity(&self, body: SolverBody) -> Option<Vec3> {
        self.backend().angular_velocity(self.handle(body.slot())?)
    }

    /// Position and rotation according to the solver — what moves for a dynamic body.
    pub fn transform(&self, body: SolverBody) -> Option<(Vec3, Quat)> {
        self.backend().get_transform(self.handle(body.slot())?)
    }

    /// Mass, for turning a spring's acceleration into the solver's impulse.
    pub fn mass(&self, body: SolverBody) -> Option<f32> {
        self.backend().mass(self.handle(body.slot())?)
    }

    /// Whether the solver has parked this body.
    pub fn is_sleeping(&self, body: SolverBody) -> Option<bool> {
        self.backend().is_sleeping(self.handle(body.slot())?)
    }
}

impl PhysicsWorld {
    /// Adds inherited shapes to the body in `slot`, untracked: they change only with a spec change,
    /// which retires the body. `pub(super)` because `Attachment` is.
    pub(super) fn attach_all(
        &mut self,
        slot: u32,
        attachments: &[super::compound::Attachment],
        meshes: Option<&ColliderMeshCache>,
    ) {
        let Some(handle) = self.handle(slot) else {
            return;
        };
        for attachment in attachments {
            // Resolved here, once per body build, rather than in the
            // per-frame walk that gathered it.
            let Some(shape) = attachment.shape(meshes) else {
                continue;
            };
            self.backend_mut().attach_collider(
                handle,
                shape,
                attachment.offset,
                attachment.rotation,
                attachment.material,
                attachment.interaction,
            );
        }
    }
}
