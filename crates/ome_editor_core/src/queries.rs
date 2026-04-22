//! ECS data gathering functions for the editor UI.

use ome_core::resource::Resources;
use ome_ecs::EphemeralComponents;
use ome_ecs::archetype::Archetype;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::component::ComponentRegistry;

use crate::state::{
    ArchetypeDisplayInfo, ComponentDisplayInfo, ComponentTypeInfo, EntityDisplayInfo,
    ReflectedTypeInfo,
};

/// Returns whether an archetype carries any marker registered as
/// ephemeral. Used to keep editor-owned entities (cameras, gizmos) out
/// of the World hierarchy and Archetype panels.
fn archetype_is_ephemeral(archetype: &Archetype, ephemeral: &EphemeralComponents) -> bool {
    archetype
        .components()
        .iter()
        .any(|tid| ephemeral.contains(tid))
}

pub(crate) fn gather_entity_data(resources: &Resources) -> Vec<EntityDisplayInfo> {
    use glam::Quat;
    use ome_ecs::hierarchy::{GlobalTransform, Parent};
    use std::collections::HashMap;

    let Some(archetypes) = resources.get::<ArchetypeRegistry>() else {
        return Vec::new();
    };
    let components = resources.get::<ComponentRegistry>();
    let ephemeral = resources
        .get::<EphemeralComponents>()
        .map(|e| e.clone())
        .unwrap_or_default();

    // Collect world-space rotations from GlobalTransform once so the
    // Inspector's World rotation display mode has a lookup table.
    let mut global_rotations: HashMap<ome_ecs::Entity, Quat> = HashMap::new();
    if let Some(registry) = components.as_ref()
        && let Some(gt_storage) = registry.get_cpu::<GlobalTransform>()
    {
        for (entity, gt) in gt_storage.iter() {
            global_rotations.insert(*entity, gt.rotation());
        }
    }

    // First pass: gather all entities with their components.
    let mut flat: Vec<EntityDisplayInfo> = Vec::new();
    let mut entity_idx_map: HashMap<ome_ecs::Entity, usize> = HashMap::new();

    for archetype in archetypes.iter_matching(&[]) {
        if archetype_is_ephemeral(archetype, &ephemeral) {
            continue;
        }
        for &entity in archetype.entities() {
            let mut comps: Vec<ComponentDisplayInfo> = archetype
                .components()
                .iter()
                .filter_map(|tid| {
                    let registry = components.as_ref()?;
                    let full_name = registry.component_name(tid)?;
                    let short_name = full_name
                        .rsplit("::")
                        .next()
                        .unwrap_or(full_name)
                        .to_owned();
                    let fields = registry.reflect_get_fields(tid, entity);
                    let field_metas = registry.reflect_field_metas(tid);
                    let visibility = registry
                        .reflect_inspector_visibility(tid)
                        .unwrap_or(ome_ecs::reflect::InspectorVisibility::Editable);
                    Some(ComponentDisplayInfo {
                        type_id: *tid,
                        short_name,
                        fields,
                        field_metas,
                        visibility,
                    })
                })
                .collect();

            // Sort: Name first, Transform second, rest alphabetically.
            comps.sort_by(|a, b| {
                fn priority(name: &str) -> u8 {
                    match name {
                        "Name" => 0,
                        "Transform" => 1,
                        _ => 2,
                    }
                }
                let pa = priority(&a.short_name);
                let pb = priority(&b.short_name);
                pa.cmp(&pb).then_with(|| a.short_name.cmp(&b.short_name))
            });

            let idx = flat.len();
            entity_idx_map.insert(entity, idx);
            flat.push(EntityDisplayInfo {
                entity,
                components: comps,
                parent: None,
                children: Vec::new(),
                depth: 0,
                global_rotation: global_rotations.get(&entity).copied(),
                parent_global_rotation: None,
            });
        }
    }

    // Second pass: populate parent/children from hierarchy components.
    if let Some(registry) = components.as_ref()
        && let Some(parent_storage) = registry.get_cpu::<Parent>()
    {
        for (child_entity, parent_comp) in parent_storage.iter() {
            if let Some(&child_idx) = entity_idx_map.get(child_entity) {
                flat[child_idx].parent = Some(parent_comp.entity);
                flat[child_idx].parent_global_rotation =
                    global_rotations.get(&parent_comp.entity).copied();
            }
            if let Some(&parent_idx) = entity_idx_map.get(&parent_comp.entity) {
                flat[parent_idx].children.push(*child_entity);
            }
        }
    }

    // Third pass: sort in tree order (roots first, then DFS children) with depth.
    // Treat entities as roots if they have no parent OR if their parent
    // doesn't exist in the entity list (e.g. Parent with Entity::INVALID).
    let roots: Vec<ome_ecs::Entity> = flat
        .iter()
        .filter(|e| match e.parent {
            None => true,
            Some(p) => !entity_idx_map.contains_key(&p),
        })
        .map(|e| e.entity)
        .collect();

    let mut sorted: Vec<EntityDisplayInfo> = Vec::with_capacity(flat.len());
    let mut stack: Vec<(ome_ecs::Entity, usize)> = Vec::new();

    // Sort roots by index for stable ordering.
    let mut sorted_roots = roots;
    sorted_roots.sort_by_key(|e| e.index());

    // Push roots in reverse so first root is processed first.
    for &root in sorted_roots.iter().rev() {
        stack.push((root, 0));
    }

    while let Some((entity, depth)) = stack.pop() {
        if let Some(&idx) = entity_idx_map.get(&entity) {
            let mut info = std::mem::replace(
                &mut flat[idx],
                EntityDisplayInfo {
                    entity: ome_ecs::Entity::INVALID,
                    components: Vec::new(),
                    parent: None,
                    children: Vec::new(),
                    depth: 0,
                    global_rotation: None,
                    parent_global_rotation: None,
                },
            );
            info.depth = depth;

            // Push children in reverse for correct DFS order.
            let mut children = info.children.clone();
            children.sort_by_key(|e| e.index());
            for &child in children.iter().rev() {
                stack.push((child, depth + 1));
            }

            sorted.push(info);
        }
    }

    sorted
}

pub(crate) fn gather_archetype_data(resources: &Resources) -> Vec<ArchetypeDisplayInfo> {
    let Some(archetypes) = resources.get::<ArchetypeRegistry>() else {
        return Vec::new();
    };
    let components = resources.get::<ComponentRegistry>();
    let ephemeral = resources
        .get::<EphemeralComponents>()
        .map(|e| e.clone())
        .unwrap_or_default();

    let mut result = Vec::new();
    for archetype in archetypes.iter_matching(&[]) {
        if archetype_is_ephemeral(archetype, &ephemeral) {
            continue;
        }
        let comp_names: Vec<String> = archetype
            .components()
            .iter()
            .map(|tid| {
                components
                    .as_ref()
                    .and_then(|r| r.component_name(tid))
                    .map(|name| name.rsplit("::").next().unwrap_or(name).to_owned())
                    .unwrap_or_else(|| format!("{:?}", tid))
            })
            .collect();

        result.push(ArchetypeDisplayInfo {
            id_short: format!("{:?}", archetype.id()),
            entity_count: archetype.len(),
            component_names: comp_names,
        });
    }
    result.sort_by(|a, b| b.entity_count.cmp(&a.entity_count));
    result
}

pub(crate) fn gather_component_types(resources: &Resources) -> Vec<ComponentTypeInfo> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let mut types: Vec<ComponentTypeInfo> = registry
        .all_type_names()
        .into_iter()
        .map(|(tid, name)| {
            let short = name.rsplit("::").next().unwrap_or(name).to_owned();
            ComponentTypeInfo {
                type_id: tid,
                short_name: short,
                has_reflection: registry.has_reflector(&tid),
            }
        })
        .collect();
    types.sort_by(|a, b| a.short_name.cmp(&b.short_name));
    types
}

pub(crate) fn gather_reflected_types(resources: &Resources) -> Vec<ReflectedTypeInfo> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let mut types: Vec<ReflectedTypeInfo> = registry
        .reflected_type_names()
        .into_iter()
        .map(|(tid, name)| {
            let short = name.rsplit("::").next().unwrap_or(name).to_owned();
            let category = registry.reflect_category(&tid);
            ReflectedTypeInfo {
                type_id: tid,
                short_name: short,
                category,
            }
        })
        .collect();
    // Sort: uncategorized first (None < Some), then by category, then by name.
    types.sort_by(|a, b| {
        a.category
            .cmp(&b.category)
            .then_with(|| a.short_name.cmp(&b.short_name))
    });
    types
}
