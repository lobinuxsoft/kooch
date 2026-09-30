//! An entity's world pose, as the solver wants it.
//!
//! 🔴 The pose is read from [`GlobalTransform`], which `PrePhysics` propagates immediately before
//! the solver is authored (#1316). This module used to walk `Parent` itself, because propagation
//! ran in `PostUpdate` — after the fixed stages — so the published global was a frame stale. The
//! walk is gone; the scale is not, and the note below says why.

use glam::{Mat4, Quat, Vec3};
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::{GlobalTransform, Parent};
use kooch_ecs::transform::Transform;

/// How deep a chain is followed before it is treated as a cycle. A hierarchy this deep is a bug
/// either way, and looping forever inside the physics sync is the worse failure.
const MAX_DEPTH: u32 = 64;

/// `entity`'s local-to-world matrix, as the last propagation published it.
fn world_of(registry: &ComponentRegistry, entity: Entity) -> Mat4 {
    if let Some(global) = registry
        .get_cpu::<GlobalTransform>()
        .and_then(|storage| storage.get(entity))
    {
        return global.matrix;
    }
    // No published global: the entity was spawned after the last propagation. Its own transform is
    // its world one only while it has no parent — and if it has one, that is worth hearing about,
    // since authoring at a local offset is the exact bug this stage was added to end.
    let parented = registry
        .get_cpu::<Parent>()
        .and_then(|storage| storage.get(entity))
        .is_some_and(|parent| parent.entity.is_valid() && parent.entity != entity);
    if parented {
        tracing::warn!(
            target: "kooch_physics",
            entity = entity.index(),
            "a parented body has no propagated GlobalTransform; authoring it at its local offset",
        );
    }
    registry
        .get_cpu::<Transform>()
        .and_then(|storage| storage.get(entity))
        .map(Transform::to_matrix)
        .unwrap_or(Mat4::IDENTITY)
}

/// `entity`'s world pose, as the solver wants it.
///
/// 🔴 The scale is the **product** of the chain's local scales, not the matrix's decomposition. A
/// decomposed scale wobbles in its last bits as the rotation moves, and the scale is part of the
/// body's spec: a wobbling spec rebuilt a spinning body every single frame. `GlobalTransform` only
/// stores a matrix, and its own `scale()` says it approximates — so this cannot come from there.
pub(super) fn world_pose(registry: &ComponentRegistry, entity: Entity) -> (Vec3, Quat, Vec3) {
    let (_, rotation, translation) = world_of(registry, entity).to_scale_rotation_translation();
    (translation, rotation, scale_of(registry, entity))
}

/// The scale `entity` is drawn at: its own, times every ancestor's.
fn scale_of(registry: &ComponentRegistry, entity: Entity) -> Vec3 {
    let transforms = registry.get_cpu::<Transform>();
    let parents = registry.get_cpu::<Parent>();
    let mut scale = transforms
        .and_then(|storage| storage.get(entity))
        .map(|transform| transform.scale)
        .unwrap_or(Vec3::ONE);
    let mut at = entity;
    for _ in 0..MAX_DEPTH {
        let Some(parent) = parents
            .and_then(|storage| storage.get(at))
            .map(|parent| parent.entity)
            .filter(|parent| parent.is_valid() && *parent != at)
        else {
            break;
        };
        if let Some(above) = transforms.and_then(|storage| storage.get(parent)) {
            scale *= above.scale;
        }
        at = parent;
    }
    scale
}

/// A world pose expressed under `entity`'s parent, which is what a local [`Transform`] holds. The
/// writeback needs it: the solver answers in world space and a parented body's transform is not.
pub(super) fn under_parent(
    registry: &ComponentRegistry,
    entity: Entity,
    position: Vec3,
    rotation: Quat,
) -> (Vec3, Quat) {
    let parent = registry
        .get_cpu::<Parent>()
        .and_then(|storage| storage.get(entity))
        .map(|parent| parent.entity)
        .filter(|parent| parent.is_valid() && *parent != entity);
    let Some(parent) = parent else {
        return (position, rotation);
    };
    // 🔴 The parent's published global, not this entity's global divided out. rapier's plugin
    // documents the same choice: deriving the parent's from the child's makes the next propagated
    // value impossible to predict through rounding, which breaks the comparison that decides
    // whether the author moved a body.
    let local =
        world_of(registry, parent).inverse() * Mat4::from_rotation_translation(rotation, position);
    let (_, rotation, translation) = local.to_scale_rotation_translation();
    (translation, rotation)
}
