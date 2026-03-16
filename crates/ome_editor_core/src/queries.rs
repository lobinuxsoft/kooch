//! ECS data gathering functions for the editor UI.

use ome_core::resource::Resources;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::component::ComponentRegistry;

use crate::state::{
    ArchetypeDisplayInfo, ComponentDisplayInfo, ComponentTypeInfo, EntityDisplayInfo,
    ReflectedTypeInfo,
};

pub(crate) fn gather_entity_data(resources: &Resources) -> Vec<EntityDisplayInfo> {
    let Some(archetypes) = resources.get::<ArchetypeRegistry>() else {
        return Vec::new();
    };
    let components = resources.get::<ComponentRegistry>();

    let mut entities = Vec::new();
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
            entities.push(EntityDisplayInfo {
                entity,
                components: comps,
            });
        }
    }
    entities.sort_by_key(|e| e.entity.index());
    entities
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
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let mut types: Vec<ReflectedTypeInfo> = registry
        .reflected_type_names()
        .into_iter()
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
