//! An entity's world pose, composed from the hierarchy rather than read from
//! [`GlobalTransform`](kooch_ecs::hierarchy::GlobalTransform).
//!
//! 🔴 The solver is authored in `PreUpdate` and propagation runs in `PostUpdate`, after the fixed
//! stages: reading the published global would be a frame stale on the first frame and wrong on it.
//! Walking `Parent` over local transforms costs the depth of the chain — two or three links for
//! anything real — and is right whatever order the stages run in (#1316).

use glam::{Mat4, Quat, Vec3};
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::Parent;
use kooch_ecs::transform::Transform;

/// How deep a chain is followed before it is treated as a cycle. A hierarchy this deep is a bug
/// either way, and looping forever inside the physics sync is the worse failure.
const MAX_DEPTH: u32 = 64;

/// `entity`'s local-to-world matrix, its own transform composed with every ancestor's.
pub(super) fn world_of(registry: &ComponentRegistry, entity: Entity) -> Mat4 {
    let transforms = registry.get_cpu::<Transform>();
    let parents = registry.get_cpu::<Parent>();
    let mut matrix = transforms
        .and_then(|storage| storage.get(entity))
        .map(Transform::to_matrix)
        .unwrap_or(Mat4::IDENTITY);
    let mut at = entity;
    for _ in 0..MAX_DEPTH {
        let Some(parent) = parents
            .and_then(|storage| storage.get(at))
            .map(|parent| parent.entity)
            .filter(|parent| parent.is_valid() && *parent != at)
        else {
            return matrix;
        };
        let Some(above) = transforms.and_then(|storage| storage.get(parent)) else {
            return matrix;
        };
        matrix = above.to_matrix() * matrix;
        at = parent;
    }
    tracing::warn!(
        target: "kooch_physics",
        entity = entity.index(),
        "a parent chain deeper than {MAX_DEPTH} was cut short; is it a cycle?",
    );
    matrix
}

/// `entity`'s world pose, as the solver wants it.
///
/// 🔴 The scale is the **product** of the chain's local scales, not the matrix's decomposition. A
/// decomposed scale wobbles in its last bits as the rotation moves, and the scale is part of the
/// body's spec: a wobbling spec rebuilt a spinning body every single frame.
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
    let local =
        world_of(registry, parent).inverse() * Mat4::from_rotation_translation(rotation, position);
    let (_, rotation, translation) = local.to_scale_rotation_translation();
    (translation, rotation)
}
