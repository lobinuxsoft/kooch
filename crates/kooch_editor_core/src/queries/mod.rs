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

/// The order components are shown in: `Name` first, `Transform` second,
/// everything else alphabetically.
///
/// Shared rather than duplicated because a prefab is inspected in the same
/// panel as an entity, and two orderings for the same list is the sort of
/// difference that is only ever noticed by the person using it.
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
            // A parked component's values live in the editor's own
            // store, so this is a clone rather than a reflection read —
            // but a clone per field per entity is the same cost in the
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

/// Everything about a component that depends on its *type* and not on
/// which entity carries it: its name, its portable id, its field
/// metadata, whether the Inspector may edit it, whether it reflects.
///
/// Every one of these was resolved once per component per entity — four
/// registry lookups and a `String` allocation, 610 times over for the
/// 610 entities of one archetype, all of them producing the same answer.
/// An archetype *is* the set of component types its entities share, so
/// this is resolved once per archetype and read per entity.
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
///
/// `wanted` is the caller's decision, not a property of the component:
/// the Inspector's selection gets its values, and every row gets its
/// `Name` because the World panel draws it.
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

/// Gathers every entity for the panels, reading reflected field values
/// only for `detail_for`.
///
/// # Why the caller decides
///
/// Reading a component's fields allocates a `String` and a `Vec` per
/// field. Across 610 entities that measured 5.26 ms per frame — 97% of
/// the gather stage, and gather is the part of the frame that a person
/// cannot make cheaper by closing a panel (#691). The values feed the
/// Inspector, which shows the selection: one entity, occasionally a few.
///
/// Everything else the panels do with a component — the hierarchy's
/// `[4]` count, the prefab marker, "does this entity have a Collider" —
/// needs the component to be *listed*, not read. Those are unaffected.
///
/// `Name` is read for every entity regardless: the World panel shows it
/// on each row, so skipping it would trade a real cost for a list of
/// "Entity 412".
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
    // A parked component is editable when *somebody* can apply the edit.
    // Over the wire that is the connected project. Locally it is the
    // editor itself: a plugin declared the type, so the schema is known
    // and `DynamicComponents` is the editor's own store to write into.
    // Without the second case, a project's components would show
    // read-only in the very mode built to author them.
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
            // Only re-sorted when there is something to merge in: the
            // archetype's own components arrived in display order, and a
            // sort per entity over an already-sorted list was 610 sorts
            // to produce the order it already had.
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

/// The rows the Components panel offers for drag-drop.
///
/// Reads the plugin registry beside the reflected one, for the same
/// reason the Add Component menu does: a project's own types have no
/// `TypeId` in this binary, so `ComponentRegistry` cannot list them.
/// This panel is a second way to do the same thing as that menu, and it
/// was still showing only what the editor was compiled with.
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
///
/// `None` in local mode, which is what makes every caller fall back to the
/// editor's own registry without a second branch.
fn remote_schema(resources: &Resources) -> Option<&[kooch_remote::protocol::ComponentSchema]> {
    let state = resources.get::<crate::remote_session::RemoteState>()?;
    if !state.is_connected() {
        return None;
    }
    state.session.as_ref().map(|s| s.schema())
}

/// The components the add-component menu offers.
///
/// **In remote mode this comes from the project, not from the editor.** The
/// project owns the components; asking the editor's own `ComponentRegistry`
/// answers with whatever that binary happened to be compiled with and omits
/// everything the project defines. That is why `RigidBody` was missing from
/// the menu until `kooch_editor_core` grew an `kooch_physics` dependency — a
/// workaround that did nothing for project-defined components.

/// Whether a component is one the engine writes rather than one a user
/// authors.
///
/// The Add Component menu is not a catalogue — the Components panel is
/// that, and it still lists every one of these so you can see they exist.
/// This is the shorter question: would adding it by hand mean anything?
///
/// For each of these it does not, and for two of them it is actively
/// harmful:
///
/// - **`Parent`** is set by dragging a row onto another, which also fixes
///   up the other side. Added by hand it is a parent reference to nothing.
/// - **`Children`** is derived from `Parent` by `hierarchy_sync_system`,
///   so whatever you put there is overwritten on the next frame.
/// - **`GlobalTransform`** is the output of transform propagation. It is
///   read constantly and authored never.
/// - **`PersistentId`** is the identity a scene file uses to point at an
///   entity. Handing out a second one is how two entities come to claim
///   the same reference.
///
/// # Matched on the name, and why
///
/// In remote mode this list arrives over the wire carrying a type name and
/// a category, and nothing else — so a name is what there is to match. The
/// `kooch_` prefix check keeps a project's own `Parent` from being caught by
/// a rule about the engine's.
///
/// A project component that wants out of the menu needs a real mechanism:
/// a `#[reflect(...)]` flag, carried as a field on the schema. Worth
/// building when something asks for it, rather than guessing at the
/// spelling now.
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

    // Types a loaded plugin declared, added in either mode. They have no
    // `TypeId` here, so the reflected registry cannot know them; and the
    // remote schema lists what the *running* project registered, which is
    // a different set from what its library declares.
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
mod engine_owned_tests {
    use super::is_engine_owned;

    /// The four the engine writes. Two of them are overwritten on the next
    /// frame; two of them break something if a second one exists.
    #[test]
    fn derived_components_are_not_offered() {
        for name in [
            "kooch_ecs::hierarchy::parent::Parent",
            "kooch_ecs::hierarchy::children::Children",
            "kooch_ecs::transform::GlobalTransform",
            "kooch_ecs::persistent_id::PersistentId",
        ] {
            assert!(
                is_engine_owned(name),
                "{name} should not be addable by hand"
            );
        }
    }

    /// Everything a user actually places has to stay in the menu. This is
    /// the assertion that fails if the list is ever widened carelessly.
    #[test]
    fn authorable_components_stay() {
        for name in [
            "kooch_ecs::transform::Transform",
            "kooch_ecs::name::Name",
            "kooch_ecs::perspective_camera::PerspectiveCamera",
            "kooch_physics::components::RigidBody",
            "kooch_camera::virtual_camera::VirtualCamera",
        ] {
            assert!(!is_engine_owned(name), "{name} must remain addable");
        }
    }

    /// A project's own `Parent` is not the engine's. The prefix check is
    /// the whole reason the rule is not just "any type called Parent".
    #[test]
    fn a_projects_own_type_is_never_caught_by_an_engine_rule() {
        assert!(!is_engine_owned("my_game::hierarchy::Parent"));
        assert!(!is_engine_owned("roll_a_ball::Children"));
    }
}
