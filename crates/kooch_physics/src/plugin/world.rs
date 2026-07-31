//! [`PhysicsWorld`] — the ECS↔backend body mapping, and the [`PhysicsBody`]
//! component that addresses it.
//!
//! # Why a slot index and not a `HashMap<Entity, BodyHandle>`
//!
//! The obvious mapping is a hash map either way round. Both directions
//! are needed every frame — spawn sync walks entities and asks "does this
//! one have a body", writeback walks bodies and asks "which entity does
//! this belong to" — and a hash map makes each of those a pointer chase
//! per element.
//!
//! Instead the world keeps parallel arrays indexed by a dense `u32` slot,
//! and the entity carries that slot as a plain POD component. Entity →
//! slot is a component lookup; slot → entity is an array index. Both
//! passes are linear walks over contiguous memory.
//!
//! The sync layer itself is not optional: Rapier owns its `RigidBodySet`
//! and `ColliderSet`, so the solver's state cannot live *in* the
//! archetypes the way an ECS-native engine would put it. That is the
//! price of the trait — and the trait is what lets a GPU solver replace
//! Rapier later without touching a single authored scene.

use glam::{Quat, Vec3};

use kooch_ecs::component::Component;
use kooch_ecs::entity::Entity;

use crate::backend::{
    BodyDesc, BodyHandle, ColliderInteraction, CollisionShape, Damping, PhysicsBackend,
    SurfaceMaterial,
};
use crate::components::{Collider, RigidBody};

/// The physics body an entity owns, as a slot into [`PhysicsWorld`].
///
/// Runtime state, not authored data: it is deliberately *not* reflected,
/// so it never reaches a scene file, the Inspector, or a
/// [`WorldSnapshot`]. Pressing stop therefore drops it along with the
/// rest of the play session, and the next sync rebuilds the physics world
/// from the restored ECS — one source of truth, no divergence between the
/// solver and the entities.
///
/// [`WorldSnapshot`]: kooch_ecs::world_snapshot::WorldSnapshot
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicsBody(u32);

impl Component for PhysicsBody {}

impl PhysicsBody {
    /// Wraps a slot index.
    pub const fn new(slot: u32) -> Self {
        Self(slot)
    }

    /// The slot this body occupies in [`PhysicsWorld`].
    pub const fn slot(&self) -> u32 {
        self.0
    }
}

/// The authored intent a body was built from.
///
/// Kept per slot so the sync pass can tell "this entity already has the
/// right body" from "this entity's body no longer matches what the
/// Inspector says", without asking the backend to describe itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodySpec {
    kind: u32,
    mass: f32,
    shape: u32,
    radius: f32,
    half_height: f32,
    half_extents: Vec3,
    /// The shape's authored centre, in the entity's local space.
    ///
    /// In the spec because it is baked into the collider when built: an
    /// offset edited in the Inspector has to *replace* the collider, and
    /// the sync pass decides that by comparing specs.
    center: Vec3,
    /// The entity's `Transform` scale, folded into the shape dimensions.
    ///
    /// Rapier's shapes take no scale — they are built from dimensions —
    /// so scaling has to happen where the shape is built, and a scale
    /// change has to *rebuild* it. Which is why this belongs in the spec:
    /// the sync pass compares specs to decide what to rebuild, so dragging
    /// the scale gizmo lands here and retires the old body.
    scale: Vec3,
    /// Digest of the shapes this body inherits from its descendants.
    ///
    /// A digest rather than the shapes themselves, so the spec stays
    /// plain-old-data and comparable by value. Adding, moving, resizing or
    /// deleting a child's collider changes it, and the sync pass rebuilds
    /// the body for the same reason it does on a scale change: rapier
    /// bakes shapes at build time.
    attachments: u64,
    /// The body's own surface, and how quickly it loses motion.
    ///
    /// In the spec for the same reason the shape is: rapier bakes both into
    /// the collider and the body at build time, so an Inspector edit has to
    /// retire and rebuild. Unlike `RigidBody::density`, the simulation
    /// genuinely reads these.
    material: SurfaceMaterial,
    interaction: ColliderInteraction,
    damping: Damping,
    /// In the spec because rapier bakes it into the body at build time.
    gravity_scale: f32,
    /// The authored centre of mass, or `None` for the collider's own.
    ///
    /// In the spec because rapier bakes mass properties into the body at
    /// build time, the same as shapes. `RigidBody::density` deliberately is
    /// *not* here: nothing in the simulation reads it, so a density edit
    /// must not retire a body and drop its velocity mid-play.
    center_of_mass: Option<Vec3>,
}

impl BodySpec {
    /// Reads the spec off the authored components.
    ///
    /// `scale` is the entity's `Transform` scale. Ignoring it was the bug
    /// that made colliders "work at some sizes and not others": the mesh
    /// grew with the gizmo and the collider stayed at its authored
    /// dimensions.
    pub fn new(body: &RigidBody, collider: &Collider, scale: Vec3) -> Self {
        Self::with_attachments(body, collider, scale, 0)
    }

    /// Same, for a body that inherits shapes from its descendants.
    pub fn with_attachments(
        body: &RigidBody,
        collider: &Collider,
        scale: Vec3,
        attachments: u64,
    ) -> Self {
        Self {
            attachments,
            kind: body.kind,
            mass: body.mass,
            shape: collider.shape,
            radius: collider.radius,
            half_height: collider.half_height,
            half_extents: collider.half_extents,
            center: collider.center,
            material: collider.material(),
            interaction: collider.interaction(),
            damping: body.damping(),
            gravity_scale: body.gravity_scale,
            center_of_mass: body.explicit_center_of_mass(),
            scale,
        }
    }

    /// The descriptor that builds this body at a given pose.
    ///
    /// # How scale folds into each shape
    ///
    /// Only a box scales exactly: its half-extents are per-axis, so a
    /// non-uniform scale is exact. The round shapes have no non-uniform
    /// form in Rapier — a non-uniformly scaled sphere is an ellipsoid, and
    /// there is no ellipsoid primitive — so they follow the convention
    /// every engine uses:
    ///
    /// - **Sphere**: radius times the largest axis scale. Enclosing the
    ///   mesh beats intersecting it; a collider smaller than what you can
    ///   see is the one that reads as a physics bug.
    /// - **Capsule**: radius times the larger of the two horizontal
    ///   scales, half-height times the vertical one, because the capsule
    ///   runs along Y.
    ///
    /// A truly non-uniform round collider wants a convex hull (#137).
    pub fn desc(&self, position: Vec3, rotation: Quat) -> BodyDesc {
        let s = self.scale.abs();
        let collider = Collider {
            shape: self.shape,
            radius: self.radius,
            half_height: self.half_height,
            half_extents: self.half_extents,
            center: self.center,
            ..Default::default()
        };
        BodyDesc {
            kind: RigidBody {
                kind: self.kind,
                mass: self.mass,
                ..Default::default()
            }
            .body_kind(),
            shape: scaled_shape(&collider, self.scale),
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
            // Body-local, and deliberately *not* pre-rotated: rapier
            // composes the body's pose on top of the collider's
            // `position_wrt_parent`, so rotating here would apply the
            // body's rotation twice.
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

/// The physics backend plus the mapping between its bodies and entities.
///
/// Slots are never compacted, so a [`PhysicsBody`] stays valid for the
/// life of its body. Freed slots go on a free list and are reused, which
/// keeps the arrays dense without invalidating anyone's index.
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
        position: Vec3,
        rotation: Quat,
    ) -> u32 {
        let handle = self.backend.add_body(spec.desc(position, rotation));
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

    /// The entity owning a backend body.
    ///
    /// A linear walk, deliberately: this runs once per reported event, and
    /// events are opt-in per collider, so the set is small by construction.
    /// An index here would be a third mapping to keep in step for a cost
    /// nobody has measured.
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

impl PhysicsWorld {
    /// Adds a body's inherited shapes to the body in `slot`.
    ///
    /// Handles are not kept: an attachment only changes as part of a spec
    /// change, and a spec change retires the whole body — which takes its
    /// shapes with it. Tracking them individually would buy nothing and
    /// give two places to forget.
    ///
    /// Visible only inside the plugin, because `Attachment` is: a `pub`
    /// signature naming a type nobody outside can name is a function that
    /// advertises itself and cannot be called.
    pub(super) fn attach_all(&mut self, slot: u32, attachments: &[super::compound::Attachment]) {
        let Some(handle) = self.handle(slot) else {
            return;
        };
        for attachment in attachments {
            self.backend_mut().attach_collider(
                handle,
                attachment.shape,
                attachment.offset,
                attachment.rotation,
                attachment.material,
                attachment.interaction,
            );
        }
    }
}

/// Folds a `Transform` scale into a collider's dimensions.
///
/// Rapier's shapes take no scale — they are built from dimensions — so
/// scaling has to happen where the shape is built. Shared by the body's
/// own shape and by the ones its descendants contribute, so a compound
/// child scales the same way the body does.
///
/// # Why the round shapes are approximations
///
/// Only a box scales exactly: its half-extents are per-axis. A
/// non-uniformly scaled sphere is an ellipsoid and Rapier has no
/// ellipsoid primitive, so the round shapes follow the convention every
/// engine uses:
///
/// - **Sphere**: radius times the largest axis scale. Enclosing the mesh
///   beats intersecting it; a collider smaller than what you can see is
///   the one that reads as a physics bug.
/// - **Capsule**: radius times the larger of the two *horizontal* scales,
///   half-height times the vertical one, because the capsule runs along Y.
///   A tall thin capsule scaled on Y should get taller, not fatter.
///
/// A truly non-uniform round collider wants a convex hull (#137).
pub(super) fn scaled_shape(collider: &Collider, scale: Vec3) -> CollisionShape {
    let s = scale.abs();
    let scaled = Collider {
        shape: collider.shape,
        radius: collider.radius * s.max_element(),
        half_height: collider.half_height * s.y,
        half_extents: collider.half_extents * s,
        center: collider.center,
        ..Default::default()
    };
    let scaled = match collider.shape {
        crate::components::SHAPE_CAPSULE => Collider {
            radius: collider.radius * s.x.max(s.z),
            ..scaled
        },
        _ => scaled,
    };
    scaled.collision_shape()
}
