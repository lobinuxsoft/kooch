//! Converting a world-space drag into the parent's space.
//!
//! A gizmo is drawn from the entity's [`GlobalTransform`], so the delta it
//! produces is in world space. A [`Transform`] is in its parent's space.
//! For a root entity the two coincide, which is why applying one to the
//! other worked for as long as nobody parented anything.
//!
//! Unlike the physics half of #612, there is no ambiguity about who owns
//! the pose here: the user dragged a handle, and the hierarchy decides
//! what that means for a child. This is arithmetic, not a design choice.

use glam::{Mat4, Quat, Vec3};

use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::{GlobalTransform, Parent};

/// The world→local transform of `entity`'s parent, or `None` when it has
/// no parent — in which case world space *is* its local space and no
/// conversion is wanted.
///
/// `None` is also returned when the parent has no `GlobalTransform` yet,
/// which happens on the frame an entity is reparented before propagation
/// runs. Treating that as "no parent" leaves the drag in world space for
/// one frame; inventing an identity matrix would teleport the entity to
/// the origin instead.
pub(super) fn parent_world_to_local(resources: &Resources, entity: Entity) -> Option<Mat4> {
    let registry = resources.get::<ComponentRegistry>()?;
    let parent = registry.get_cpu::<Parent>()?.get(entity)?.entity;
    let parent_world = registry.get_cpu::<GlobalTransform>()?.get(parent)?.matrix;
    Some(parent_world.inverse())
}

/// Converts a world-space translation delta into the parent's space.
///
/// Only the rotation and scale of the parent matter — a translation delta
/// is a direction and a length, not a point, so the parent's own position
/// must not be added to it.
pub(super) fn translation_to_parent_space(world_to_local: Mat4, delta: Vec3) -> Vec3 {
    world_to_local.transform_vector3(delta)
}

/// Converts a world-space rotation delta into the parent's space.
///
/// The delta is applied by left-multiplication onto the entity's local
/// rotation, so it has to be expressed in the same space that rotation
/// lives in: conjugated by the parent's world rotation.
pub(super) fn rotation_to_parent_space(world_to_local: Mat4, delta: Quat) -> Quat {
    let (_, parent_inverse_rotation, _) = world_to_local.to_scale_rotation_translation();
    (parent_inverse_rotation * delta * parent_inverse_rotation.inverse()).normalize()
}

#[cfg(test)]
mod tests;
