//! ECS data gathering functions for the editor UI.

use ome_core::resource::Resources;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::component::ComponentRegistry;

use crate::state::{
    ArchetypeDisplayInfo, ComponentDisplayInfo, ComponentTypeInfo, EntityDisplayInfo,
    ReflectedTypeInfo,
};

pub(crate) fn gather_entity_data(resources: &Resources) -> Vec<EntityDisplayInfo> {
    use ome_ecs::hierarchy::Parent;
    use std::collections::HashMap;

    let Some(archetypes) = resources.get::<ArchetypeRegistry>() else {
        return Vec::new();
    };
    let components = resources.get::<ComponentRegistry>();

    // First pass: gather all entities with their components.
    let mut flat: Vec<EntityDisplayInfo> = Vec::new();
    let mut entity_idx_map: HashMap<ome_ecs::Entity, usize> = HashMap::new();

    for archetype in archetypes.iter_matching(&[]) {
        for &entity in archetype.entities() {
            let comps: Vec<ComponentDisplayInfo> = archetype
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
                    Some(ComponentDisplayInfo {
                        type_id: *tid,
                        short_name,
                        fields,
                    })
                })
                .collect();

            let idx = flat.len();
            entity_idx_map.insert(entity, idx);
            flat.push(EntityDisplayInfo {
                entity,
                components: comps,
                parent: None,
                children: Vec::new(),
                depth: 0,
            });
        }
    }

    // Second pass: populate parent/children from hierarchy components.
    if let Some(registry) = components.as_ref() {
        if let Some(parent_storage) = registry.get_cpu::<Parent>() {
            for (child_entity, parent_comp) in parent_storage.iter() {
                if let Some(&child_idx) = entity_idx_map.get(child_entity) {
                    flat[child_idx].parent = Some(parent_comp.entity);
                }
                if let Some(&parent_idx) = entity_idx_map.get(&parent_comp.entity) {
                    flat[parent_idx].children.push(*child_entity);
                }
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

    let mut result = Vec::new();
    for archetype in archetypes.iter_matching(&[]) {
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
    use ome_ecs::hierarchy::{Children, GlobalTransform, Parent};

    // Hierarchy components are system-managed, not user-addable.
    let hidden = [
        std::any::TypeId::of::<Parent>(),
        std::any::TypeId::of::<Children>(),
        std::any::TypeId::of::<GlobalTransform>(),
    ];

    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let mut types: Vec<ReflectedTypeInfo> = registry
        .reflected_type_names()
        .into_iter()
        .filter(|(tid, _)| !hidden.contains(tid))
        .map(|(tid, name)| {
            let short = name.rsplit("::").next().unwrap_or(name).to_owned();
            ReflectedTypeInfo {
                type_id: tid,
                short_name: short,
            }
        })
        .collect();
    types.sort_by(|a, b| a.short_name.cmp(&b.short_name));
    types
}
