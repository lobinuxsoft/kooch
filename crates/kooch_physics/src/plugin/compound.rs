//! A child [`Collider`] without [`PhysicsBody`] joins the nearest ancestor body — solver and
//! hierarchy cannot both own a pose. A descendant with its own [`PhysicsBody`] is independent and
//! ends the walk.

use glam::{Quat, Vec3};
use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::{Children, GlobalTransform};

use crate::backend::{ColliderInteraction, ColliderMeshCache, CollisionShape, SurfaceMaterial};
use crate::components::{Collider, PhysicsBody, ShapeSpec};

/// A descendant's shape in body space, authored not resolved: gathered every frame, and [`digest`]
/// compares `Copy` fields.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Attachment {
    pub spec: ShapeSpec,
    /// The child's scale relative to the body, in the digest since it changes shape with pose
    /// unchanged.
    pub scale: Vec3,
    pub offset: Vec3,
    pub rotation: Quat,
    /// The child's own surface. An ice patch welded onto a crate is still
    /// ice — the body's material has no business overriding it.
    pub material: SurfaceMaterial,
    /// The child's own filtering and event opt-ins. A trigger volume
    /// parented to a crate is still a trigger volume.
    pub interaction: ColliderInteraction,
}

impl Attachment {
    /// Geometry at its own scale; `None` while a mesh waits — its epoch rebuilds the body when it
    /// lands.
    pub fn shape(&self, meshes: Option<&ColliderMeshCache>) -> Option<CollisionShape> {
        Some(self.spec.resolve(meshes)?.scaled(self.scale))
    }
}

/// Shapes a body inherits: depth-first, stopping at any [`PhysicsBody`], poses composed through
/// [`GlobalTransform`] relative to `root`.
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
    // The project's table, or the one every collider had before it existed.
    let owned_layers;
    let layers = match resources.get::<kooch_core::layers::LayerNames>() {
        Some(layers) => layers,
        None => {
            owned_layers = kooch_core::layers::LayerNames::default();
            &owned_layers
        }
    };
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
            found.push(Attachment {
                // No cache here on purpose: the epoch belongs in the
                // digest, and this walk runs for every body every frame.
                // The resolve that needs it happens once, at attach.
                spec: collider.shape_spec(entity, None),
                scale,
                offset: translation + collider.center,
                rotation,
                material: collider.material(),
                interaction: collider.in_layers(layers).interaction(),
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

/// A stable digest of the attachments, in [`BodySpec`](super::world::BodySpec) so a child
/// collider's change retires the body like a scale change, keeping the spec POD.
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
        (i.is_trigger, i.collision_events, i.contact_force_events).hash(&mut hasher);
        i.contact_force_threshold.to_bits().hash(&mut hasher);
        let mask = i.collision_groups;
        (mask.memberships, mask.filter).hash(&mut hasher);
        hash_spec(&attachment.spec, &mut hasher);
        // A child scaled in place changes its shape while its offset and
        // rotation stay exactly where they were.
        for value in attachment.scale.to_array() {
            value.to_bits().hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Hashes float bits: a shape moved by one ulp did move.
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

/// Warns that a nested dynamic body will not follow its parent, as Godot does; static and kinematic
/// children are author-driven anyway.
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
