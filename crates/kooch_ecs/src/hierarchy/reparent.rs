//! Reparent math — keeps an entity's world-space TRS invariant when its `Parent` component changes.

use std::any::TypeId;

use crate::archetype_registry::ArchetypeRegistry;
use crate::component::ComponentRegistry;
use crate::entity::Entity;
use crate::hierarchy::Parent;
use crate::transform::Transform;
use glam::{Quat, Vec3};
use kooch_core::resource::Resources;

/// Rewrites an entity's local `Transform` so its world-space TRS
/// stays the same across a reparent. Call this BEFORE updating the
/// entity's `Parent` component.
pub(super) fn rewrite_local_transform_for_reparent(
    resources: &mut Resources,
    entity: Entity,
    new_parent: Option<Entity>,
) {
    let Some((child_wp, child_wr, child_ws)) = compute_world_trs(resources, entity) else {
        return;
    };
    let (parent_wp, parent_wr, parent_ws) = match new_parent {
        Some(p) => {
            compute_world_trs(resources, p).unwrap_or((Vec3::ZERO, Quat::IDENTITY, Vec3::ONE))
        }
        None => (Vec3::ZERO, Quat::IDENTITY, Vec3::ONE),
    };

    // Inverse of `world = T + R · (S ⊙ local)`: subtract T, apply R⁻¹, then divide by S
    // component-wise.
    let parent_rot_inv = parent_wr.inverse();
    let inv_parent_scale = Vec3::new(
        safe_inv(parent_ws.x),
        safe_inv(parent_ws.y),
        safe_inv(parent_ws.z),
    );
    let new_local_pos = (parent_rot_inv * (child_wp - parent_wp)) * inv_parent_scale;
    let new_local_rot = parent_rot_inv * child_wr;
    let new_local_scale = child_ws * inv_parent_scale;

    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(transform_storage) = registry.get_cpu_mut::<Transform>()
        && let Some(transform) = transform_storage.get_mut(entity)
    {
        transform.position = new_local_pos;
        transform.rotation = new_local_rot;
        transform.scale = new_local_scale;
    }
}

/// Walks up the parent chain from `entity` to a root, composing TRS per component. Returns the
/// world-space `(translation, rotation, scale)` or `None` if the entity has no `Transform`.
fn compute_world_trs(resources: &Resources, entity: Entity) -> Option<(Vec3, Quat, Vec3)> {
    let registry = resources.get::<ComponentRegistry>()?;
    let transform_storage = registry.get_cpu::<Transform>()?;
    let parent_storage = registry.get_cpu::<Parent>();

    let mut chain = Vec::with_capacity(8);
    chain.push(entity);
    let mut current = entity;
    while let Some(parent) = parent_storage.as_ref().and_then(|s| s.get(current)) {
        if chain.contains(&parent.entity) {
            break;
        }
        chain.push(parent.entity);
        current = parent.entity;
    }
    chain.reverse();

    let mut world_pos = Vec3::ZERO;
    let mut world_rot = Quat::IDENTITY;
    let mut world_scale = Vec3::ONE;
    for &e in &chain {
        let t = transform_storage.get(e)?;
        let new_pos = world_pos + world_rot * (world_scale * t.position);
        let new_rot = world_rot * t.rotation;
        let new_scale = world_scale * t.scale;
        world_pos = new_pos;
        world_rot = new_rot;
        world_scale = new_scale;
    }
    Some((world_pos, world_rot, world_scale))
}

/// Inverse with a floor to avoid division by zero on degenerate scales.
fn safe_inv(v: f32) -> f32 {
    if v.abs() < 1e-6 { 1.0 / 1e-6 } else { 1.0 / v }
}

/// Reparents `entity` under `new_parent`, or unparents it when `None`.
pub fn reparent(resources: &mut Resources, entity: Entity, new_parent: Option<Entity>) {
    // Preserve the child's world-space transform across the reparent. Without this, parenting snaps
    // the child to `parent * child_local` and unparenting snaps it back to `child_local` (as if it
    // were a root all along).
    rewrite_local_transform_for_reparent(resources, entity, new_parent);

    match new_parent {
        Some(parent) => {
            let mut needs_archetype_add = false;
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                let has_parent = registry
                    .get_cpu::<Parent>()
                    .is_some_and(|s| s.contains(entity));
                if has_parent {
                    if let Some(storage) = registry.get_cpu_mut::<Parent>()
                        && let Some(p) = storage.get_mut(entity)
                    {
                        p.entity = parent;
                    }
                } else if let Some(storage) = registry.get_cpu_mut::<Parent>() {
                    storage.insert(entity, Parent { entity: parent });
                    needs_archetype_add = true;
                }
            }
            if needs_archetype_add {
                let parent_tid = TypeId::of::<Parent>();
                if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
                    && let Some(current) = archetypes.entity_archetype(entity)
                {
                    let new_arch = archetypes.archetype_after_add_dynamic(current, parent_tid);
                    archetypes.register_entity(entity, new_arch);
                }
            }
        }
        None => {
            let parent_tid = TypeId::of::<Parent>();
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                registry.remove_component(entity, &parent_tid);
            }
            if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
                && let Some(current) = archetypes.entity_archetype(entity)
            {
                let new_arch = archetypes.archetype_after_remove_dynamic(current, parent_tid);
                archetypes.register_entity(entity, new_arch);
            }
        }
    }
}
