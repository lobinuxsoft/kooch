//! Gathering a body's extra shapes from its descendants.
//!
//! A child entity carrying a [`Collider`] but no [`RigidBody`] of its own
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
//! Godot has had "allow a RigidBody to follow a moving parent" open for
//! years. The way out is to stop having two bodies.
//!
//! A descendant that *does* carry its own [`RigidBody`] is left alone — it
//! is an independent body, and joining two bodies is what a
//! [`Joint`](crate::components::Joint) is for. It also ends the walk:
//! entities under it belong to that body, not to this one.

use glam::{Quat, Vec3};
use ome_core::resource::Resources;
use ome_ecs::component::ComponentRegistry;
use ome_ecs::entity::Entity;
use ome_ecs::hierarchy::{Children, GlobalTransform};

use crate::backend::{ColliderInteraction, CollisionShape, SurfaceMaterial};
use crate::components::{Collider, RigidBody};

use super::world::scaled_shape;

/// One shape contributed by a descendant, in the body's local space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Attachment {
    pub shape: CollisionShape,
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
/// [`RigidBody`]. Poses are expressed relative to `root` by composing
/// through [`GlobalTransform`], so a child's own parent chain is honoured
/// however deep it goes.
pub(super) fn attachments_for(resources: &Resources, root: Entity) -> Vec<Attachment> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let (Some(children), Some(globals)) = (
        registry.get_cpu::<Children>(),
        registry.get_cpu::<GlobalTransform>(),
    ) else {
        return Vec::new();
    };
    let colliders = registry.get_cpu::<Collider>();
    let bodies = registry.get_cpu::<RigidBody>();

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
            found.push(Attachment {
                shape: scaled_shape(collider, scale),
                offset: translation + collider.center,
                rotation,
                material: collider.material(),
                interaction: collider.interaction(),
            });
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
        match attachment.shape {
            CollisionShape::Sphere { radius } => {
                0u8.hash(&mut hasher);
                radius.to_bits().hash(&mut hasher);
            }
            CollisionShape::Cuboid { half_extents } => {
                1u8.hash(&mut hasher);
                for value in half_extents.to_array() {
                    value.to_bits().hash(&mut hasher);
                }
            }
            CollisionShape::Capsule {
                radius,
                half_height,
            } => {
                2u8.hash(&mut hasher);
                radius.to_bits().hash(&mut hasher);
                half_height.to_bits().hash(&mut hasher);
            }
        }
    }
    hasher.finish()
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
fn warn_nested_body(entity: Entity, body: &RigidBody) {
    use crate::backend::BodyKind;

    if body.body_kind() != BodyKind::Dynamic {
        return;
    }
    tracing::warn!(
        target: "ome_physics",
        entity = entity.index(),
        "a dynamic RigidBody under another body does not follow its parent — \
         the solver owns its pose. For one body with several shapes, remove \
         this RigidBody and keep the Collider; to link two bodies that both \
         simulate, add a Joint component naming them both",
    );
}
