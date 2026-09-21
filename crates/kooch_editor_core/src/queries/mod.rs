//! ECS data gathering functions for the editor UI.

use kooch_core::resource::Resources;
use kooch_ecs::EphemeralComponents;
use kooch_ecs::archetype::Archetype;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::component::{ComponentId, ComponentNames, ComponentRegistry, DynamicTypeRegistry};
use kooch_ecs::dynamic_components::DynamicComponents;
use kooch_ecs::reflect::InspectorVisibility;

use crate::state::{
    ArchetypeDisplayInfo, ComponentDisplayInfo, ComponentTypeInfo, EntityDisplayInfo,
    ReflectedFields, ReflectedTypeInfo,
};

/// Resolves a component's interned identity for a DTO.
fn component_id(names: Option<&ComponentNames>, full_name: &str) -> ComponentId {
    names
        .and_then(|n| n.id(full_name))
        .unwrap_or(ComponentId::INVALID)
}

/// The order components are shown in: `Name` first, `Transform` second, everything else
/// alphabetically.
pub(crate) fn display_order(a: &str, b: &str) -> std::cmp::Ordering {
    fn priority(name: &str) -> u8 {
        match name {
            "Name" => 0,
            "Transform" => 1,
            _ => 2,
        }
    }
    priority(a).cmp(&priority(b)).then_with(|| a.cmp(b))
}

/// Stand-in type handle for a component this binary has no Rust type for. Such a component is
/// addressed only by its [`ComponentId`]; the `TypeId` slot in the DTO exists for the
/// reflection-facing paths, which never fire for a parked component.
struct ParkedComponent;

/// Interns every component name the UI can display — the registry's own types plus any parked in
/// [`DynamicComponents`] — so the read-only gather pass can resolve each to a [`ComponentId`]. Runs
/// before gathering.
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
    // Types declared by a loaded plugin. Without interning these, the
    // menu would list them with `ComponentId::INVALID` and every attempt
    // to add one would be dropped as unresolvable.
    if let Some(types) = resources.get::<DynamicTypeRegistry>() {
        names.extend(types.iter().map(|ty| ty.type_name.clone()));
    }
    // The connected project's schema. Without this the add-component menu
    // could only offer what the editor binary was compiled with, and a
    // component the project defines would have no `ComponentId` to carry.
    if let Some(schema) = remote_schema(resources) {
        names.extend(schema.iter().map(|c| c.type_name.clone()));
    }
    if let Some(interner) = resources.get_mut::<ComponentNames>() {
        for name in &names {
            interner.intern(name);
        }
    }
}

/// Builds display entries for the components parked under `entity`.
fn parked_components(
    dynamic: &DynamicComponents,
    names: Option<&ComponentNames>,
    entity: kooch_ecs::Entity,
    editable: bool,
    detailed: bool,
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
            short_name: std::borrow::Cow::Owned(
                full_name
                    .rsplit("::")
                    .next()
                    .unwrap_or(full_name)
                    .to_owned(),
            ),
            // A parked component's values live in the editor's own store, so this is a clone rather
            // than a reflection read — but a clone per field per entity is the same cost in the
            // same place, and the Inspector is the only reader.
            fields: if detailed {
                ReflectedFields::Values(fields.to_vec())
            } else {
                ReflectedFields::NotGathered
            },
            field_metas: None,
            visibility,
        })
        .collect()
}

/// Everything about a component that depends on its *type* and not on which entity carries it: its
/// name, its portable id, its field metadata, whether the Inspector may edit it, whether it
/// reflects.
struct ComponentMeta {
    type_id: std::any::TypeId,
    component: ComponentId,
    short_name: &'static str,
    field_metas: Option<&'static [kooch_ecs::reflect::FieldMeta]>,
    visibility: InspectorVisibility,
    /// Whether the type is registered for reflection. Distinct from
    /// "has values this frame": the schema does not depend on who is
    /// selected.
    reflected: bool,
}

/// The component types of one archetype, resolved and in display order,
/// plus what the World panel needs to know about the whole set.
struct ArchetypeMeta {
    components: Vec<ComponentMeta>,
    /// Whether these entities belong to a prefab instance. A property of
    /// the archetype: `PrefabMember` is either in the signature or it is
    /// not.
    is_prefab_instance: bool,
}

impl ArchetypeMeta {
    fn resolve(
        archetype: &Archetype,
        registry: Option<&ComponentRegistry>,
        names: Option<&ComponentNames>,
    ) -> Self {
        let mut components: Vec<ComponentMeta> = archetype
            .components()
            .iter()
            .filter_map(|tid| {
                let registry = registry?;
                let full_name = registry.component_name(tid)?;
                Some(ComponentMeta {
                    type_id: *tid,
                    component: component_id(names, full_name),
                    short_name: full_name.rsplit("::").next().unwrap_or(full_name),
                    field_metas: registry.reflect_field_metas(tid),
                    visibility: registry
                        .reflect_inspector_visibility(tid)
                        .unwrap_or(InspectorVisibility::Editable),
                    reflected: registry.has_reflector(tid),
                })
            })
            .collect();
        components.sort_by(|a, b| display_order(a.short_name, b.short_name));
        let is_prefab_instance = components.iter().any(|c| c.short_name == "PrefabMember");
        Self {
            components,
            is_prefab_instance,
        }
    }
}

/// Reads one component's field values, or says why they are not here.
fn reflected_fields(
    registry: Option<&ComponentRegistry>,
    meta: &ComponentMeta,
    entity: kooch_ecs::Entity,
    wanted: bool,
) -> ReflectedFields {
    if !meta.reflected {
        return ReflectedFields::Unreflected;
    }
    if !wanted {
        return ReflectedFields::NotGathered;
    }
    match registry.and_then(|r| r.reflect_get_fields(&meta.type_id, entity)) {
        Some(values) => ReflectedFields::Values(values),
        // Registered for reflection but the read came back empty — the
        // entity does not actually hold this component. Not
        // "unreflectable": the type's schema is fine.
        None => ReflectedFields::NotGathered,
    }
}

/// Returns whether an archetype carries any marker registered as ephemeral. Used to keep
/// editor-owned entities (cameras, gizmos) out of the World hierarchy and Archetype panels.
fn archetype_is_ephemeral(archetype: &Archetype, ephemeral: &EphemeralComponents) -> bool {
    let mirror = std::any::TypeId::of::<crate::remote_mirror::MirrorEntity>();
    archetype
        .components()
        .iter()
        .any(|tid| *tid != mirror && ephemeral.contains(tid))
}

/// Gathers every entity for the panels, reading reflected field values only for `detail_for`.
pub(crate) fn gather_entity_data(
    resources: &Resources,
    detail_for: &std::collections::HashSet<kooch_ecs::Entity>,
) -> Vec<EntityDisplayInfo> {
    use glam::Quat;
    use kooch_ecs::hierarchy::{GlobalTransform, Parent};
    use std::collections::HashMap;

    let Some(archetypes) = resources.get::<ArchetypeRegistry>() else {
        return Vec::new();
    };
    let components = resources.get::<ComponentRegistry>();
    let names = resources.get::<ComponentNames>();
    // Components with no local Rust type, shown alongside the real ones.
    let dynamic = resources.get::<DynamicComponents>();
    // A parked component is editable when *somebody* can apply the edit. Over the wire that is the
    // connected project. Locally it is the editor itself: a plugin declared the type, so the schema
    // is known and `DynamicComponents` is the editor's own store to write into.
    let parked_editable = resources
        .get::<crate::remote_session::RemoteState>()
        .is_some_and(|s| s.is_connected())
        || resources
            .get::<DynamicTypeRegistry>()
            .is_some_and(|types| !types.is_empty());
    let ephemeral = resources
        .get::<EphemeralComponents>()
        .map(|e| e.clone())
        .unwrap_or_default();

    // Scene membership, so the World panel can group rows by the file
    // they came from.
    let scene_of: HashMap<kooch_ecs::Entity, kooch_core::Guid> = components
        .as_ref()
        .and_then(|registry| registry.get_cpu::<kooch_ecs::SceneMember>())
        .map(|storage| storage.iter().map(|(&e, m)| (e, m.scene)).collect())
        .unwrap_or_default();

    // Collect world-space rotations from GlobalTransform once so the
    // Inspector's World rotation display mode has a lookup table.
    let mut global_rotations: HashMap<kooch_ecs::Entity, Quat> = HashMap::new();
    if let Some(registry) = components.as_ref()
        && let Some(gt_storage) = registry.get_cpu::<GlobalTransform>()
    {
        for (entity, gt) in gt_storage.iter() {
            global_rotations.insert(*entity, gt.rotation());
        }
    }

    // First pass: gather all entities with their components.
    let mut flat: Vec<EntityDisplayInfo> = Vec::new();
    let mut entity_idx_map: HashMap<kooch_ecs::Entity, usize> = HashMap::new();

    for archetype in archetypes.iter_matching(&[]) {
        if archetype_is_ephemeral(archetype, &ephemeral) {
            continue;
        }
        let meta = ArchetypeMeta::resolve(archetype, components.as_deref(), names);

        for &entity in archetype.entities() {
            let detailed = detail_for.contains(&entity);
            let mut comps: Vec<ComponentDisplayInfo> = meta
                .components
                .iter()
                .map(|c| ComponentDisplayInfo {
                    type_id: c.type_id,
                    component: c.component,
                    short_name: std::borrow::Cow::Borrowed(c.short_name),
                    fields: reflected_fields(
                        components.as_deref(),
                        c,
                        entity,
                        detailed || c.short_name == "Name",
                    ),
                    field_metas: c.field_metas,
                    visibility: c.visibility,
                })
                .collect();

            let parked = dynamic.as_ref().map(|dynamic| {
                parked_components(dynamic, names, entity, parked_editable, detailed)
            });
            // Only re-sorted when there is something to merge in: the archetype's own components
            // arrived in display order, and a sort per entity over an already-sorted list was 610
            // sorts to produce the order it already had.
            if let Some(parked) = parked.filter(|p| !p.is_empty()) {
                comps.extend(parked);
                comps.sort_by(|a, b| display_order(&a.short_name, &b.short_name));
            }

            let idx = flat.len();
            entity_idx_map.insert(entity, idx);
            flat.push(EntityDisplayInfo {
                entity,
                // Read off the component the editor's instancing attaches,
                // so the World panel can offer Revert without a world.
                is_prefab_instance: meta.is_prefab_instance,
                components: comps,
                parent: None,
                children: Vec::new(),
                depth: 0,
                global_rotation: global_rotations.get(&entity).copied(),
                scene: scene_of.get(&entity).copied(),
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
    let roots: Vec<kooch_ecs::Entity> = flat
        .iter()
        .filter(|e| match e.parent {
            None => true,
            Some(p) => !entity_idx_map.contains_key(&p),
        })
        .map(|e| e.entity)
        .collect();

    let mut sorted: Vec<EntityDisplayInfo> = Vec::with_capacity(flat.len());
    let mut stack: Vec<(kooch_ecs::Entity, usize)> = Vec::new();

    // 🔴 By `Order` first, `Entity::index` second. The index alone was never a decision anybody made
    // — it is the order the allocator handed slots out in, which the panel and the scene file both
    // read and therefore agreed on by coincidence.
    let order_of = |e: &kooch_ecs::Entity| {
        (
            components
                .as_ref()
                .and_then(|r| r.get_cpu::<kooch_ecs::Order>())
                .and_then(|s| s.get(*e))
                .map(|o| o.value),
            e.index(),
        )
    };
    let key = |e: &kooch_ecs::Entity| {
        let (order, index) = order_of(e);
        // `None` last: `Option`'s own ordering puts it first, which would
        // float every unordered entity to the top of its group.
        (order.is_none(), order.unwrap_or(0), index)
    };

    let mut sorted_roots = roots;
    sorted_roots.sort_by_key(&key);

    // Push roots in reverse so first root is processed first.
    for &root in sorted_roots.iter().rev() {
        stack.push((root, 0));
    }

    while let Some((entity, depth)) = stack.pop() {
        if let Some(&idx) = entity_idx_map.get(&entity) {
            let mut info = std::mem::replace(
                &mut flat[idx],
                EntityDisplayInfo {
                    scene: None,
                    entity: kooch_ecs::Entity::INVALID,
                    is_prefab_instance: false,
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
            children.sort_by_key(&key);
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

/// The rows the Components panel offers for drag-drop.
pub(crate) fn gather_component_types(resources: &Resources) -> Vec<ComponentTypeInfo> {
    let names = resources.get::<ComponentNames>();
    let mut types: Vec<ComponentTypeInfo> = resources
        .get::<ComponentRegistry>()
        .map(|registry| {
            registry
                .all_type_names()
                .into_iter()
                .map(|(tid, name)| ComponentTypeInfo {
                    component: component_id(names, name),
                    short_name: name.rsplit("::").next().unwrap_or(name).to_owned(),
                    has_reflection: registry.has_reflector(&tid),
                })
                .collect()
        })
        .unwrap_or_default();

    if let Some(dynamic) = resources.get::<DynamicTypeRegistry>() {
        for ty in dynamic.iter() {
            let already = types.iter().any(|t| {
                names
                    .and_then(|n| n.name(t.component))
                    .is_some_and(|name| name == ty.type_name)
            });
            if already {
                continue;
            }
            types.push(ComponentTypeInfo {
                component: component_id(names, &ty.type_name),
                short_name: ty
                    .type_name
                    .rsplit("::")
                    .next()
                    .unwrap_or(&ty.type_name)
                    .to_owned(),
                // Its schema is known, which is what "reflected" means to
                // this panel: the row can be dragged onto an entity and
                // the Inspector can draw the fields it lands as.
                has_reflection: true,
            });
        }
    }

    types.sort_by(|a, b| a.short_name.cmp(&b.short_name));
    types
}

/// The connected project's component schema, if there is one.
pub(crate) fn remote_schema(
    resources: &Resources,
) -> Option<&[kooch_remote::protocol::ComponentSchema]> {
    let state = resources.get::<crate::remote_session::RemoteState>()?;
    if !state.is_connected() {
        return None;
    }
    state.session.as_ref().map(|s| s.schema())
}

/// The components the add-component menu offers.

/// Whether a component is one the engine writes rather than one a user authors.
fn is_engine_owned(type_name: &str) -> bool {
    const DERIVED: &[&str] = &["Parent", "Children", "GlobalTransform", "PersistentId"];

    let Some(short) = type_name.rsplit("::").next() else {
        return false;
    };
    type_name.starts_with("kooch_") && DERIVED.contains(&short)
}

pub(crate) fn gather_reflected_types(resources: &Resources) -> Vec<ReflectedTypeInfo> {
    let names = resources.get::<ComponentNames>();

    let mut types: Vec<ReflectedTypeInfo> = match remote_schema(resources) {
        Some(schema) => schema
            .iter()
            .filter(|component| !is_engine_owned(&component.type_name))
            .map(|component| ReflectedTypeInfo {
                component: component_id(names, &component.type_name),
                short_name: component
                    .type_name
                    .rsplit("::")
                    .next()
                    .unwrap_or(&component.type_name)
                    .to_owned(),
                category: component.category.clone(),
            })
            .collect(),
        None => {
            let local: Vec<ReflectedTypeInfo> = resources
                .get::<ComponentRegistry>()
                .map(|registry| {
                    registry
                        .reflected_type_names()
                        .into_iter()
                        .filter(|(_, name)| !is_engine_owned(name))
                        .map(|(tid, name)| ReflectedTypeInfo {
                            component: component_id(names, name),
                            short_name: name.rsplit("::").next().unwrap_or(name).to_owned(),
                            category: registry.reflect_category(&tid).map(str::to_owned),
                        })
                        .collect()
                })
                .unwrap_or_default();

            local
        }
    };

    // Types a loaded plugin declared, added in either mode. They have no `TypeId` here, so the
    // reflected registry cannot know them; and the remote schema lists what the *running* project
    // registered, which is a different set from what its library declares.
    if let Some(dynamic) = resources.get::<DynamicTypeRegistry>() {
        for ty in dynamic.iter() {
            // Skip anything the wire already reported, or the same
            // component appears twice under two categories.
            let already = types.iter().any(|t| {
                names
                    .and_then(|n| n.name(t.component))
                    .is_some_and(|name| name == ty.type_name)
            });
            if already {
                continue;
            }
            types.push(ReflectedTypeInfo {
                component: component_id(names, &ty.type_name),
                short_name: ty
                    .type_name
                    .rsplit("::")
                    .next()
                    .unwrap_or(&ty.type_name)
                    .to_owned(),
                // Grouped by the plugin that brought them, so a project's
                // components do not scatter through the engine's own list.
                category: Some(ty.source.clone()),
            });
        }
    }

    // Sort: uncategorized first (None < Some), then by category, then by name.
    types.sort_by(|a, b| {
        a.category
            .cmp(&b.category)
            .then_with(|| a.short_name.cmp(&b.short_name))
    });
    types
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod engine_owned_tests;
