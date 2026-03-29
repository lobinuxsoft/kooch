//! Transform propagation system — computes GlobalTransform from hierarchy.

use glam::Mat4;

use crate::entity::Entity;

use super::children::Children;
use super::global_transform::GlobalTransform;
use super::parent::Parent;

/// Propagates transforms top-down through the hierarchy.
///
/// Computes `GlobalTransform` for all entities with `Transform`:
/// - Roots (no `Parent`): `GlobalTransform = Transform::to_matrix()`
/// - Children: `GlobalTransform = parent.GlobalTransform * local.to_matrix()`
pub fn transform_propagation_system(resources: &mut ome_core::resource::Resources) {
    use crate::component::ComponentRegistry;
    use crate::transform::Transform;

    let Some(mut registry) = resources.remove::<ComponentRegistry>() else {
        return;
    };

    // Ensure GlobalTransform storage exists.
    registry.register_cpu_reflected::<GlobalTransform>();

    // Gather all entities with Transform and their Parent (if any).
    let transform_entities: Vec<(Entity, Transform, Option<Entity>)> = {
        let transform_storage = registry.get_cpu::<Transform>();
        let parent_storage = registry.get_cpu::<Parent>();

        match transform_storage {
            Some(ts) => ts
                .iter()
                .map(|(entity, transform)| {
                    let parent = parent_storage
                        .as_ref()
                        .and_then(|ps| ps.get(*entity))
                        .map(|p| p.entity);
                    (*entity, *transform, parent)
                })
                .collect(),
            None => {
                resources.insert(registry);
                return;
            }
        }
    };

    // Identify roots (no Parent or parent has no Transform).
    let roots: Vec<Entity> = transform_entities
        .iter()
        .filter(|(_, _, parent)| parent.is_none())
        .map(|(e, _, _)| *e)
        .collect();

    // Build entity→Transform lookup.
    let transform_map: std::collections::HashMap<Entity, Transform> = transform_entities
        .iter()
        .map(|(e, t, _)| (*e, *t))
        .collect();

    // BFS propagation.
    let mut queue: std::collections::VecDeque<(Entity, Mat4)> = std::collections::VecDeque::new();

    // Seed roots.
    for &root in &roots {
        let matrix = transform_map[&root].to_matrix();
        queue.push_back((root, matrix));
    }

    // Get Children storage for traversal (read-only snapshot of entity lists).
    let children_map: std::collections::HashMap<Entity, Vec<Entity>> = registry
        .get_cpu::<Children>()
        .map(|s| {
            s.iter()
                .map(|(e, c)| (*e, c.entities.clone()))
                .collect()
        })
        .unwrap_or_default();

    let mut global_transforms: Vec<(Entity, GlobalTransform)> = Vec::new();

    while let Some((entity, parent_global)) = queue.pop_front() {
        global_transforms.push((entity, GlobalTransform { matrix: parent_global }));

        if let Some(children) = children_map.get(&entity) {
            for &child in children {
                if let Some(child_transform) = transform_map.get(&child) {
                    let child_global = parent_global * child_transform.to_matrix();
                    queue.push_back((child, child_global));
                }
            }
        }
    }

    // Write GlobalTransform values.
    for (entity, gt) in global_transforms {
        if let Some(storage) = registry.get_cpu_mut::<GlobalTransform>() {
            if let Some(existing) = storage.get_mut(entity) {
                *existing = gt;
            } else {
                storage.insert(entity, gt);
            }
        }
    }

    resources.insert(registry);
}
