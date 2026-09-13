//! Converting a world-space drag into the parent's space.

use glam::{Mat4, Quat, Vec3};

use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::{GlobalTransform, Parent};

/// The world→local transform of `entity`'s parent, or `None` when it has no parent — in which case
/// world space *is* its local space and no conversion is wanted.
pub(super) fn parent_world_to_local(resources: &Resources, entity: Entity) -> Option<Mat4> {
    let registry = resources.get::<ComponentRegistry>()?;
    let parent = registry.get_cpu::<Parent>()?.get(entity)?.entity;
    let parent_world = registry.get_cpu::<GlobalTransform>()?.get(parent)?.matrix;
    Some(parent_world.inverse())
}

/// Converts a world-space translation delta into the parent's space.
pub(super) fn translation_to_parent_space(world_to_local: Mat4, delta: Vec3) -> Vec3 {
    world_to_local.transform_vector3(delta)
}

/// Converts a world-space rotation delta into the parent's space.
pub(super) fn rotation_to_parent_space(world_to_local: Mat4, delta: Quat) -> Quat {
    let (_, parent_inverse_rotation, _) = world_to_local.to_scale_rotation_translation();
    (parent_inverse_rotation * delta * parent_inverse_rotation.inverse()).normalize()
}

#[cfg(test)]
mod tests;
