//! Transform propagation system — computes GlobalTransform from hierarchy.

use std::any::TypeId;

use glam::Mat4;

use crate::archetype_registry::ArchetypeRegistry;
use crate::entity::Entity;

use super::children::Children;
use super::global_transform::GlobalTransform;
use super::parent::Parent;

/// Propagates transforms top-down through the hierarchy.
pub fn transform_propagation_system(resources: &mut kooch_core::resource::Resources) {
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
        .map(|s| s.iter().map(|(e, c)| (*e, c.entities.clone())).collect())
        .unwrap_or_default();

    let mut global_transforms: Vec<(Entity, GlobalTransform)> = Vec::new();

    while let Some((entity, parent_global)) = queue.pop_front() {
        global_transforms.push((
            entity,
            GlobalTransform {
                matrix: parent_global,
            },
        ));

        if let Some(children) = children_map.get(&entity) {
            for &child in children {
                if let Some(child_transform) = transform_map.get(&child) {
                    let child_global = parent_global * child_transform.to_matrix();
                    queue.push_back((child, child_global));
                }
            }
        }
    }

    // Write GlobalTransform values. Track which entities gained the
    // component for the first time so their archetype can be updated.
    let mut gained_gt: Vec<Entity> = Vec::new();
    for (entity, gt) in global_transforms {
        if let Some(storage) = registry.get_cpu_mut::<GlobalTransform>() {
            if let Some(existing) = storage.get_mut(entity) {
                *existing = gt;
            } else {
                storage.insert(entity, gt);
                gained_gt.push(entity);
            }
        }
    }

    resources.insert(registry);

    // Sync archetypes for entities that gained GlobalTransform. Without
    // this, the component sits in storage but no archetype lists it,
    // which means queries iterating by archetype never see it.
    if !gained_gt.is_empty()
        && let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
    {
        let gt_tid = TypeId::of::<GlobalTransform>();
        for entity in gained_gt {
            if let Some(current) = archetypes.entity_archetype(entity) {
                let new_arch = archetypes.archetype_after_add_dynamic(current, gt_tid);
                archetypes.register_entity(entity, new_arch);
            }
        }
    }
}

/// Publishes the `GlobalTransform` of `roots` and everything under them, and nothing else.
///
/// 🔴 For the case where a handful of entities moved and the world did not: physics interpolation
/// writes the drawn pose of the dynamic bodies, and whatever reads a target needs that published
/// before it looks (#1423). A full pass rebuilds two maps over every entity in the scene — 2159 of
/// them in `dense.scene` — to republish the half-dozen that changed.
///
/// A root's own global is its parent's PUBLISHED global times its local: the parent did not move
/// this step, so that value is current. A root with no parent is its local.
pub fn propagate_subtrees(resources: &mut kooch_core::resource::Resources, roots: &[Entity]) {
    use crate::component::ComponentRegistry;
    use crate::transform::Transform;

    if roots.is_empty() {
        return;
    }
    let Some(mut registry) = resources.remove::<ComponentRegistry>() else {
        return;
    };

    let published: Vec<(Entity, GlobalTransform)> = {
        let (Some(transforms), Some(globals)) = (
            registry.get_cpu::<Transform>(),
            registry.get_cpu::<GlobalTransform>(),
        ) else {
            resources.insert(registry);
            return;
        };
        let parents = registry.get_cpu::<Parent>();
        let children = registry.get_cpu::<Children>();

        let mut out = Vec::new();
        let mut queue: std::collections::VecDeque<(Entity, Mat4)> =
            std::collections::VecDeque::new();

        for &root in roots {
            let Some(local) = transforms.get(root) else {
                continue;
            };
            // The parent's published global, not a recomputed one: it did not move this step, and
            // recomputing it is the full pass this exists to avoid.
            let above = parents
                .and_then(|storage| storage.get(root))
                .map(|parent| parent.entity)
                .filter(|parent| parent.is_valid() && *parent != root)
                .and_then(|parent| globals.get(parent))
                .map(|global| global.matrix)
                .unwrap_or(Mat4::IDENTITY);
            queue.push_back((root, above * local.to_matrix()));
        }

        // 🔴 Bounded by the subtree, not by the scene. A cycle in the hierarchy would still spin
        // here, so the visited set is what makes that a wasted pass rather than a hang.
        let mut seen = std::collections::HashSet::new();
        while let Some((entity, matrix)) = queue.pop_front() {
            if !seen.insert(entity) {
                continue;
            }
            out.push((entity, GlobalTransform { matrix }));
            let Some(below) = children.and_then(|storage| storage.get(entity)) else {
                continue;
            };
            for &child in &below.entities {
                if let Some(local) = transforms.get(child) {
                    queue.push_back((child, matrix * local.to_matrix()));
                }
            }
        }
        out
    };

    if let Some(storage) = registry.get_cpu_mut::<GlobalTransform>() {
        for (entity, global) in published {
            // Only entities that already HAVE one: gaining the component needs an archetype move,
            // and a body that has never been propagated is the full pass's job, not this one's.
            if let Some(existing) = storage.get_mut(entity) {
                *existing = global;
            }
        }
    }
    resources.insert(registry);
}

#[cfg(test)]
mod subtree_tests;
