//! ECS data gathering functions for the editor UI.

use ome_core::resource::Resources;
use ome_ecs::EphemeralComponents;
use ome_ecs::archetype::Archetype;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::component::{ComponentId, ComponentNames, ComponentRegistry};
use ome_ecs::dynamic_components::DynamicComponents;
use ome_ecs::reflect::InspectorVisibility;

use crate::state::{
    ArchetypeDisplayInfo, ComponentDisplayInfo, ComponentTypeInfo, EntityDisplayInfo,
    ReflectedTypeInfo,
};

/// Resolves a component's interned identity for a DTO.
///
/// Read-only: [`ComponentNames`] is pre-populated with every registry
/// name by [`intern_registry_names`] before the gather pass, so a known
/// component always resolves. An unseen name yields
/// [`ComponentId::INVALID`], which downstream actions treat as
/// unresolvable rather than misapplying.
fn component_id(names: Option<&ComponentNames>, full_name: &str) -> ComponentId {
    names
        .and_then(|n| n.id(full_name))
        .unwrap_or(ComponentId::INVALID)
}

/// Stand-in type handle for a component this binary has no Rust type
/// for. Such a component is addressed only by its [`ComponentId`]; the
/// `TypeId` slot in the DTO exists for the reflection-facing paths,
/// which never fire for a parked component.
struct ParkedComponent;

/// Interns every component name the UI can display — the registry's own
/// types plus any parked in [`DynamicComponents`] — so the read-only
/// gather pass can resolve each to a [`ComponentId`]. Runs before
/// gathering.
pub(crate) fn intern_registry_names(resources: &mut Resources) {
    let mut names: Vec<String> = resources
        .get::<ComponentRegistry>()
        .map(|r| {
            r.all_type_names()
                .into_iter()
                .map(|(_, name)| name.to_owned())
                .collect()
        })
        .unwrap_or_default();
    if let Some(dynamic) = resources.get::<DynamicComponents>() {
        names.extend(dynamic.type_names().map(str::to_owned));
    }
    if let Some(interner) = resources.get_mut::<ComponentNames>() {
        for name in &names {
            interner.intern(name);
        }
    }
}

/// Builds display entries for the components parked under `entity`.
///
/// A parked component is one the loader met by name but this binary has
/// no Rust type for — a project's own component seen by the standalone
/// hub, chiefly. It has no reflector, so its fields come straight from
/// the store and it carries no field metadata.
///
/// `editable` gates the fields: an edit is only deliverable when a
/// remote session owns the type and can apply it. Locally there is
/// nothing to write to, so the component shows read-only rather than
/// offering widgets whose edits get dropped.
fn parked_components(
    dynamic: &DynamicComponents,
    names: Option<&ComponentNames>,
    entity: ome_ecs::Entity,
    editable: bool,
) -> Vec<ComponentDisplayInfo> {
    let visibility = if editable {
        InspectorVisibility::Editable
    } else {
        InspectorVisibility::ReadOnly
    };
    dynamic
        .iter_entity(entity)
        .map(|(full_name, fields)| ComponentDisplayInfo {
            type_id: std::any::TypeId::of::<ParkedComponent>(),
            component: component_id(names, full_name),
            short_name: full_name
                .rsplit("::")
                .next()
                .unwrap_or(full_name)
                .to_owned(),
            fields: Some(fields.to_vec()),
            field_metas: None,
            visibility,
        })
        .collect()
}

/// Returns whether an archetype carries any marker registered as
/// ephemeral. Used to keep editor-owned entities (cameras, gizmos) out
/// of the World hierarchy and Archetype panels.
///
/// `MirrorEntity` is the deliberate exception: it is ephemeral for
/// *saves* (a mirrored world belongs to the remote project, not to the
/// editor's scene file) but must stay visible — in remote mode the
/// mirror **is** the entire contents of the World panel.
fn archetype_is_ephemeral(archetype: &Archetype, ephemeral: &EphemeralComponents) -> bool {
    let mirror = std::any::TypeId::of::<crate::remote_mirror::MirrorEntity>();
    archetype
        .components()
        .iter()
        .any(|tid| *tid != mirror && ephemeral.contains(tid))
}

pub(crate) fn gather_entity_data(resources: &Resources) -> Vec<EntityDisplayInfo> {
    use glam::Quat;
    use ome_ecs::hierarchy::{GlobalTransform, Parent};
    use std::collections::HashMap;

    let Some(archetypes) = resources.get::<ArchetypeRegistry>() else {
        return Vec::new();
    };
    let components = resources.get::<ComponentRegistry>();
    let names = resources.get::<ComponentNames>();
    // Components with no local Rust type, shown alongside the real ones.
    let dynamic = resources.get::<DynamicComponents>();
    let parked_editable = resources
        .get::<crate::remote_session::RemoteState>()
        .is_some_and(|s| s.is_connected());
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
                        component: component_id(names, full_name),
                        short_name,
                        fields,
                        field_metas,
                        visibility,
                    })
                })
                .collect();

            if let Some(dynamic) = dynamic.as_ref() {
                comps.extend(parked_components(dynamic, names, entity, parked_editable));
            }

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
    let names = resources.get::<ComponentNames>();
    let mut types: Vec<ComponentTypeInfo> = registry
        .all_type_names()
        .into_iter()
        .map(|(tid, name)| {
            let short = name.rsplit("::").next().unwrap_or(name).to_owned();
            ComponentTypeInfo {
                component: component_id(names, name),
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
    let names = resources.get::<ComponentNames>();
    let mut types: Vec<ReflectedTypeInfo> = registry
        .reflected_type_names()
        .into_iter()
        .map(|(tid, name)| {
            let short = name.rsplit("::").next().unwrap_or(name).to_owned();
            let category = registry.reflect_category(&tid);
            ReflectedTypeInfo {
                component: component_id(names, name),
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

#[cfg(test)]
mod tests {
    use ome_ecs::allocator::EntityAllocator;
    use ome_ecs::commands::Commands;
    use ome_ecs::query::AccessTracker;
    use ome_ecs::reflect::ReflectValue;
    use ome_ecs::transform::Transform;
    use ome_remote::protocol::{ComponentSnapshot, EntityId, EntitySnapshot};

    use crate::remote_mirror::{MirrorEntity, RemoteMirror};

    use super::*;

    /// A mirrored world: ECS resources plus the ephemeral registration
    /// the editor plugin performs at startup.
    fn mirrored_world() -> Resources {
        let mut r = Resources::new();
        r.insert(EntityAllocator::new());
        r.insert(ComponentRegistry::new());
        r.insert(ArchetypeRegistry::new());
        r.insert(AccessTracker::new());
        r.insert(Commands::new());
        r.insert(DynamicComponents::new());
        r.insert(ComponentNames::new());
        r.get_mut::<ComponentRegistry>()
            .unwrap()
            .register_cpu_reflected::<Transform>();

        let mut ephemeral = EphemeralComponents::new();
        ephemeral.insert(std::any::TypeId::of::<MirrorEntity>());
        r.insert(ephemeral);

        let snapshot = vec![EntitySnapshot {
            id: EntityId {
                index: 0,
                generation: 0,
            },
            name: Some("Mesh".into()),
            parent: None,
            components: vec![
                ComponentSnapshot {
                    type_name: std::any::type_name::<Transform>().into(),
                    fields: vec![],
                },
                ComponentSnapshot {
                    type_name: "game::spin::Spin".into(),
                    fields: vec![("rpm".into(), ReflectValue::F32(33.0))],
                },
            ],
        }];
        RemoteMirror::new().apply(&snapshot, &mut r);
        r
    }

    /// Mirrored entities stay visible even though `MirrorEntity` is
    /// ephemeral — in remote mode they are the whole World panel.
    #[test]
    fn mirrored_entities_are_not_filtered_as_ephemeral() {
        let mut resources = mirrored_world();
        intern_registry_names(&mut resources);
        assert_eq!(gather_entity_data(&resources).len(), 1);
    }

    /// A component with no local Rust type still reaches the Inspector,
    /// with its field values, read-only while no session can apply an edit.
    #[test]
    fn parked_components_surface_read_only_without_a_session() {
        let mut resources = mirrored_world();
        intern_registry_names(&mut resources);

        let entities = gather_entity_data(&resources);
        let spin = entities[0]
            .components
            .iter()
            .find(|c| c.short_name == "Spin")
            .expect("parked component displayed");

        assert_eq!(spin.visibility, InspectorVisibility::ReadOnly);
        assert_ne!(spin.component, ComponentId::INVALID);
        assert_eq!(
            spin.fields.as_deref(),
            Some(&[("rpm".to_owned(), ReflectValue::F32(33.0))][..])
        );
    }
}
