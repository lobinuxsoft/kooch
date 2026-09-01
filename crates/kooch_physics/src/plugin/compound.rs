//! Gathering a body's extra shapes from its descendants.
//!
//! A child entity carrying a [`Collider`] but no [`PhysicsBody`] of its own
//! contributes its shape to the nearest ancestor that has one. The result
//! is one body with several shapes — Unity calls it a compound collider,
//! Unreal calls it welding.
//!
//! # Why not one body per collider
//!
//! Because then the solver and the transform hierarchy would both own the
//! child's pose, and no engine supports that. Unity tells you to put a
//! single Rigidbody on the root; Unreal welds simulated children into the
//! parent and its own tracker notes that bodies detach when both simulate;
//! Godot has had "allow a PhysicsBody to follow a moving parent" open for
//! years. The way out is to stop having two bodies.
//!
//! A descendant that *does* carry its own [`PhysicsBody`] is left alone — it
//! is an independent body, and joining two bodies is what a
//! [`Joint`](crate::components::Joint) is for. It also ends the walk:
//! entities under it belong to that body, not to this one.

use glam::{Quat, Vec3};
use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::{Children, GlobalTransform};

use crate::backend::{ColliderInteraction, ColliderMeshCache, CollisionShape, SurfaceMaterial};
use crate::components::{Collider, PhysicsBody, ShapeSpec};

use super::world::scaled_shape;

/// One shape contributed by a descendant, in the body's local space.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Attachment {
    pub shape: CollisionShape,
    /// The same geometry as plain old data, for [`digest`].
    ///
    /// Hashing the shape itself would walk every vertex of every child's
    /// trimesh, every frame, to answer a question the spec answers in
    /// thirteen `Copy` fields.
    pub spec: ShapeSpec,
    pub offset: Vec3,
    pub rotation: Quat,
    /// The child's own surface. An ice patch welded onto a crate is still
    /// ice — the body's material has no business overriding it.
    pub material: SurfaceMaterial,
    /// The child's own filtering and event opt-ins. A trigger volume
    /// parented to a crate is still a trigger volume.
    pub interaction: ColliderInteraction,
}

/// Collects the shapes a body inherits from its descendants.
///
/// Walks children depth-first, stopping at any entity with its own
/// [`PhysicsBody`]. Poses are expressed relative to `root` by composing
/// through [`GlobalTransform`], so a child's own parent chain is honoured
/// however deep it goes.
pub(super) fn attachments_for(resources: &Resources, root: Entity) -> Vec<Attachment> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let meshes = resources.get::<ColliderMeshCache>();
    let (Some(children), Some(globals)) = (
        registry.get_cpu::<Children>(),
        registry.get_cpu::<GlobalTransform>(),
    ) else {
        return Vec::new();
    };
    let colliders = registry.get_cpu::<Collider>();
    let bodies = registry.get_cpu::<PhysicsBody>();

    // The root's world pose, inverted once: every descendant's local pose
    // is its world pose seen from here.
    let Some(root_world) = globals.get(root) else {
        return Vec::new();
    };
    let to_local = root_world.matrix.inverse();

    let mut found = Vec::new();
    let mut stack: Vec<Entity> = children
        .get(root)
        .map(|c| c.entities.clone())
        .unwrap_or_default();

    while let Some(entity) = stack.pop() {
        // Its own body: an independent simulation. Not ours, and neither
        // is anything beneath it.
        if let Some(nested) = bodies.and_then(|storage| storage.get(entity)) {
            warn_nested_body(entity, nested);
            continue;
        }

        if let Some(collider) = colliders.and_then(|storage| storage.get(entity))
            && let Some(world) = globals.get(entity)
        {
            let local = to_local * world.matrix;
            let (scale, rotation, translation) = local.to_scale_rotation_translation();
            // A child still waiting for its mesh contributes nothing this
            // frame. Its epoch is in the digest, so the body rebuilds —
            // and picks the shape up — the moment the mesh lands.
            if let Some(shape) = scaled_shape(collider, scale, meshes) {
                found.push(Attachment {
                    shape,
                    spec: collider.shape_spec(meshes),
                    offset: translation + collider.center,
                    rotation,
                    material: collider.material(),
                    interaction: collider.interaction(),
                });
            }
        }

        if let Some(grandchildren) = children.get(entity) {
            stack.extend(grandchildren.entities.iter().copied());
        }
    }

    // Hash-map iteration order is not stable between runs, and shape
    // creation order is observable in the solver. Sort so two runs of the
    // same scene agree.
    found.sort_unstable_by(|a, b| {
        a.offset
            .to_array()
            .partial_cmp(&b.offset.to_array())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    found
}

/// A stable digest of a body's attachments.
///
/// Lives in [`BodySpec`](super::world::BodySpec) so the existing
/// retire-and-rebuild pass notices when a child's collider is added,
/// removed, moved or resized — the same way it already notices a scale
/// change on the body itself. Keeps the spec plain-old-data instead of
/// growing a `Vec`.
pub(super) fn digest(attachments: &[Attachment]) -> u64 {
    use std::hash::{Hash, Hasher};

    // Zero means "inherits nothing", so a body with no descendants
    // compares equal to one built without ever asking. Hashing the empty
    // slice would give some other constant and make the two disagree.
    if attachments.is_empty() {
        return 0;
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    attachments.len().hash(&mut hasher);
    for attachment in attachments {
        // Floats have no Hash; their bits do, and bit equality is the
        // right test here — a shape that moved by one ulp did move.
        for value in attachment
            .offset
            .to_array()
            .into_iter()
            .chain(attachment.rotation.to_array())
        {
            value.to_bits().hash(&mut hasher);
        }
        // The surface is part of the shape's identity for rebuild
        // purposes: editing a child's friction has to reach the solver,
        // and rapier bakes it at build time like everything else.
        attachment.material.friction.to_bits().hash(&mut hasher);
        attachment.material.restitution.to_bits().hash(&mut hasher);
        (attachment.material.friction_rule as u8).hash(&mut hasher);
        (attachment.material.restitution_rule as u8).hash(&mut hasher);
        // Filtering and event opt-ins are baked into the collider too, so
        // an edit to either has to rebuild the body.
        let i = attachment.interaction;
        (i.sensor, i.collision_events, i.contact_force_events).hash(&mut hasher);
        i.contact_force_threshold.to_bits().hash(&mut hasher);
        for mask in [i.collision_groups, i.solver_groups] {
            (mask.memberships, mask.filter).hash(&mut hasher);
        }
        hash_spec(&attachment.spec, &mut hasher);
    }
    hasher.finish()
}

/// A shape's authored identity, hashed field by field.
///
/// Floats have no `Hash`; their bits do, and bit equality is the right
/// test here — a shape that moved by one ulp did move.
fn hash_spec(spec: &ShapeSpec, hasher: &mut impl std::hash::Hasher) {
    use std::hash::Hash;

    spec.shape.hash(hasher);
    for value in [
        spec.radius,
        spec.half_height,
        spec.border_radius,
        spec.voxel_size,
    ]
    .into_iter()
    .chain(spec.half_extents.to_array())
    .chain(spec.normal.to_array())
    .chain(spec.point_a.to_array())
    .chain(spec.point_b.to_array())
    .chain(spec.point_c.to_array())
    {
        value.to_bits().hash(hasher);
    }
    spec.voxel_solid.hash(hasher);
    spec.mesh.hash(hasher);
    // What makes a mesh *arriving* reach the body that was authored
    // before it: the GUID never changed, so nothing else here would.
    spec.mesh_epoch.hash(hasher);
}

/// Warns that a body nested under another will not follow its parent.
///
/// This is the configuration no engine supports, and saying so is the
/// engine's job — silently simulating it somewhere the author did not
/// expect is worse than refusing. Godot warns on the node for the same
/// reason.
///
/// Only dynamic bodies are worth warning about. A static or kinematic
/// child is author-driven anyway, so "the solver ignores your parent" is
/// not news: nothing was going to move it but the author.
fn warn_nested_body(entity: Entity, body: &PhysicsBody) {
    use crate::backend::BodyKind;

    if body.body_kind() != BodyKind::Dynamic {
        return;
    }
    tracing::warn!(
        target: "kooch_physics",
        entity = entity.index(),
        "a dynamic PhysicsBody under another body does not follow its parent — \
         the solver owns its pose. For one body with several shapes, remove \
         this PhysicsBody and keep the Collider; to link two bodies that both \
         simulate, add a Joint component naming them both",
    );
}
