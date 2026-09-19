use crate::allocator::EntityAllocator;
use crate::archetype_registry::ArchetypeRegistry;
use crate::commands::Commands;
use crate::component::ComponentRegistry;
use crate::dynamic_components::DynamicComponents;
use kooch_core::resource::Resources;

use super::document::SceneDocument;
use super::entity_refs::{DeferredRef, resolve_deferred};
use super::error::SceneError;

/// Clears the live ECS and rebuilds it from a [`SceneDocument`].
pub fn sync_scene_to_ecs(
    scene: &SceneDocument,
    resources: &mut Resources,
) -> Result<(), SceneError> {
    despawn_all(resources);
    spawn_scene_into(scene, resources)
}

/// Spawns a document's entities beside whatever is already loaded.
pub fn spawn_scene_into(
    scene: &SceneDocument,
    resources: &mut Resources,
) -> Result<(), SceneError> {
    spawn_scene_as(scene, resources, scene.id)
}

/// Spawns a document's entities as the scene instance `instance`.
pub fn spawn_scene_as(
    scene: &SceneDocument,
    resources: &mut Resources,
    instance: kooch_core::Guid,
) -> Result<(), SceneError> {
    spawn_returning_as(scene, resources, instance).map(|_| ())
}

/// Stamps out a copy of `prefab` inside the scene `into`, and hands back its root.
pub fn instantiate(
    prefab: &SceneDocument,
    resources: &mut Resources,
    into: kooch_core::Guid,
) -> Result<crate::entity::Entity, SceneError> {
    let (root, _) = instantiate_members(prefab, resources, into)?;
    Ok(root)
}

/// Instances `prefab` and hands back its root **and** every entity it spawned, in document order.
pub fn instantiate_members(
    prefab: &SceneDocument,
    resources: &mut Resources,
    into: kooch_core::Guid,
) -> Result<(crate::entity::Entity, Vec<crate::entity::Entity>), SceneError> {
    use crate::persistent_id::PersistentIdAllocator;

    // Checked before anything is spawned: a multi-root document would
    // otherwise leave its entities in the world with no root to hand back,
    // and the caller has no way to undo a partial spawn.
    let root = prefab.root_index()?;

    // Created on demand — a hand-built `Resources` (tests, headless tools)
    // will not have had `EcsPlugin` insert one.
    if resources.get::<PersistentIdAllocator>().is_none() {
        resources.insert(PersistentIdAllocator::new());
    }
    let instance = {
        let mut allocator = resources
            .remove::<PersistentIdAllocator>()
            .expect("just inserted");
        let instance = prefab.as_instance_of(into, &mut allocator);
        resources.insert(allocator);
        instance
    };

    let spawned = spawn_returning(&instance, resources)?;
    // `spawn_returning` pushes one entity per description, in order, so the
    // index the root was found at addresses the same entity here.
    Ok((spawned[root], spawned))
}

/// Shared body of [`spawn_scene_into`] and [`instantiate`], handing back
/// the entities it spawned in document order.
fn spawn_returning(
    scene: &SceneDocument,
    resources: &mut Resources,
) -> Result<Vec<crate::entity::Entity>, SceneError> {
    spawn_returning_as(scene, resources, scene.id)
}

/// [`spawn_returning`], into a named scene instance.
fn spawn_returning_as(
    scene: &SceneDocument,
    resources: &mut Resources,
    instance: kooch_core::Guid,
) -> Result<Vec<crate::entity::Entity>, SceneError> {
    use crate::hierarchy::Parent;

    // Identity has to be a known type before the spawn pass, or the ids in the file get parked as
    // an unknown component and every reference resolves to nothing. Registering here rather than
    // relying on `EcsPlugin` keeps a hand-built `Resources` loading correctly.
    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.register_cpu_reflected::<crate::persistent_id::PersistentId>();
    }

    // A fresh report per top-level load. A nested prefab shares the outer
    // one, or a prefab placed 600 times says the same thing 600 times.
    let nested = resources
        .get::<InstancingChain>()
        .is_some_and(|chain| !chain.0.is_empty());
    if !nested {
        // Taken, not read: an unset source belongs to whoever set it
        // last, and reporting a stale path sends the user to the wrong
        // file.
        let source = resources
            .remove::<LoadSource>()
            .map(|source| source.0)
            .unwrap_or_else(|| scene.id.to_string());
        resources.insert(Reported::new(source));
    }

    // First pass: spawn entities and insert components.
    // Track name → Entity for parent resolution.
    let mut name_to_entity: std::collections::HashMap<String, crate::entity::Entity> =
        std::collections::HashMap::new();
    let mut spawned_order: Vec<crate::entity::Entity> = Vec::new();
    // References cannot be written while spawning: the entity a reference
    // points at may not exist yet, and one pointing forwards would resolve
    // to nothing purely because of document order.
    let mut deferred: Vec<DeferredRef> = Vec::new();

    for entity_desc in &scene.entities {
        // A description carrying `PrefabInstance` is a *reference*: the scene did not store this
        // entity's components, the prefab has them. Building it means instancing the prefab and
        // then applying what the user changed.
        let entity = match instance_source(entity_desc) {
            Some(source) => rebuild_instance(entity_desc, source, resources, instance),
            None => {
                let mut commands = resources
                    .remove::<Commands>()
                    .expect("Commands not found in Resources");
                let entity = commands.spawn(resources).id();
                resources.insert(commands);
                entity
            }
        };

        name_to_entity.insert(entity_desc.name.clone(), entity);
        spawned_order.push(entity);
        tag_with_scene(resources, entity, instance);

        for comp_desc in &entity_desc.components {
            // Look up the TypeId by full type name.
            let type_id = {
                let components = resources.get::<ComponentRegistry>();
                components.and_then(|c| c.type_id_by_name(&comp_desc.type_name))
            };
            let Some(type_id) = type_id else {
                // `EcsPlugin` inserts the store, but a hand-built
                // `Resources` (tests, headless tools) may not have it.
                // Create it on demand rather than dropping user data.
                if resources.get::<DynamicComponents>().is_none() {
                    resources.insert(DynamicComponents::new());
                }
                if let Some(dynamic) = resources.get_mut::<DynamicComponents>() {
                    dynamic.insert(entity, &comp_desc.type_name, comp_desc.fields.clone());
                }
                report_type(resources, &comp_desc.type_name);
                continue;
            };

            // Insert default component via reflection.
            {
                let mut inserted = false;
                if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                    inserted = registry.insert_default_reflected(&type_id, entity);
                }
                if inserted {
                    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                        if let Some(current) = archetypes.entity_archetype(entity) {
                            let new_arch = archetypes.archetype_after_add_dynamic(current, type_id);
                            archetypes.register_entity(entity, new_arch);
                        }
                    }
                }
            }

            // Set each field value.
            for (field_name, value) in &comp_desc.fields {
                // An unresolved reference waits for the second pass.
                // Writing it now would be rejected by `reflect_set`, and
                // rightly so — the handle it needs does not exist yet.
                if let crate::reflect::ReflectValue::EntityRef(Some(reference)) = value
                    && reference.is_unresolved()
                {
                    deferred.push(DeferredRef {
                        entity,
                        type_id,
                        field: field_name.clone(),
                        reference: *reference,
                    });
                    continue;
                }
                let wrote = match resources.get_mut::<ComponentRegistry>() {
                    Some(registry) => {
                        registry.reflect_set_field(&type_id, entity, field_name, value.clone())
                    }
                    None => Ok(()),
                };
                if let Err(error) = wrote {
                    report_field(resources, &comp_desc.type_name, field_name, error);
                }
            }
        }
    }

    // 3. Second pass: rebuild the hierarchy of *legacy* scenes only.
    let parent_tid = std::any::TypeId::of::<Parent>();
    for (index, entity) in spawned_order.iter().enumerate() {
        let desc = &scene.entities[index];
        let resolved = match desc.parent_index {
            Some(parent_index) => spawned_order.get(parent_index).copied(),
            // Legacy scenes carry a name instead. Ambiguous by construction,
            // so say so rather than picking one silently — which is the bug
            // this replaces.
            None => desc.parent.as_ref().and_then(|name| {
                let matches = scene.entities.iter().filter(|e| &e.name == name).count();
                if matches > 1 {
                    tracing::warn!(
                        target: "kooch_ecs::scene",
                        %name,
                        matches,
                        "legacy scene names an ambiguous parent; re-save to fix",
                    );
                }
                name_to_entity.get(name).copied()
            }),
        };
        {
            if let Some(parent_entity) = resolved {
                if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                    registry.register_cpu_reflected::<Parent>();
                    if let Some(storage) = registry.get_cpu_mut::<Parent>() {
                        storage.insert(
                            *entity,
                            Parent {
                                entity: parent_entity,
                            },
                        );
                    }
                }
                // Update the archetype to include Parent.
                if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                    if let Some(current) = archetypes.entity_archetype(*entity) {
                        let new_arch = archetypes.archetype_after_add_dynamic(current, parent_tid);
                        archetypes.register_entity(*entity, new_arch);
                    }
                }
            }
        }
    }

    // Resolve entity references now that every entity exists.
    resolve_deferred(resources, deferred);

    // `Parent` is the authoritative side and `Children` is derived from it by a system — which has
    // not run yet. Anything reading the hierarchy between here and the next frame sees a tree with
    // no branches: capturing a freshly instanced prefab gave back its root alone.
    rebuild_children(&spawned_order, resources);

    Ok(spawned_order)
}

/// Prefabs currently being instanced, innermost last.
#[derive(Default)]
struct InstancingChain(Vec<kooch_core::Guid>);

/// Where the next scene spawned came from.
struct LoadSource(String);

/// Tells the next load which file it is reading.
pub fn loading_from(resources: &mut Resources, path: &std::path::Path) {
    resources.insert(LoadSource(path.display().to_string()));
}

/// What this load has already complained about.
struct Reported {
    source: String,
    types: std::collections::HashSet<String>,
    fields: std::collections::HashSet<String>,
}

impl Reported {
    fn new(source: String) -> Self {
        Self {
            source,
            types: std::collections::HashSet::new(),
            fields: std::collections::HashSet::new(),
        }
    }
}

/// Says a type did not resolve, once per type per load.
fn report_type(resources: &mut Resources, type_name: &str) {
    let Some(reported) = resources.get_mut::<Reported>() else {
        return;
    };
    if !reported.types.insert(type_name.to_owned()) {
        return;
    }
    let scene = reported.source.clone();
    tracing::warn!(
        target: "kooch_ecs::scene",
        scene,
        component = %type_name,
        "no type by that name in this build; the component is parked and \
         written back untouched, but nothing will run it",
    );
}

/// Says a stored value did not land, once per field per load.
fn report_field(
    resources: &mut Resources,
    type_name: &str,
    field: &str,
    error: crate::reflect::ReflectError,
) {
    let key = format!("{type_name}.{field}");
    let Some(reported) = resources.get_mut::<Reported>() else {
        return;
    };
    if !reported.fields.insert(key.clone()) {
        return;
    }
    let scene = reported.source.clone();
    match error {
        crate::reflect::ReflectError::FieldNotFound(_) => tracing::debug!(
            target: "kooch_ecs::scene",
            scene,
            field = %key,
            "the component has no such field any more; the stored value is dropped",
        ),
        error => tracing::warn!(
            target: "kooch_ecs::scene",
            scene,
            field = %key,
            "a stored value did not load: {error}",
        ),
    }
}

/// Despawns only the entities belonging to `scene`.
pub fn despawn_scene(scene: kooch_core::Guid, resources: &mut Resources) {
    use crate::scene_member::SceneMember;

    let doomed: Vec<crate::entity::Entity> = resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<SceneMember>())
        .map(|storage| {
            storage
                .iter()
                .filter(|(_, member)| member.scene == scene)
                .map(|(&entity, _)| entity)
                .collect()
        })
        .unwrap_or_default();

    despawn_entities(resources, &doomed);
}

/// Despawns every alive entity in the ECS, except those marked ephemeral.
fn despawn_all(resources: &mut Resources) {
    use crate::ephemeral::EphemeralComponents;

    // Snapshot ephemeral markers; default to empty if the resource is
    // not present (e.g., headless tests without an editor plugin).
    let ephemeral = resources
        .get::<EphemeralComponents>()
        .map(|e| e.clone())
        .unwrap_or_default();

    // Collect all alive entities from archetypes, skipping ephemeral ones.
    let entities: Vec<_> = resources
        .get::<ArchetypeRegistry>()
        .map(|archetypes| {
            archetypes
                .iter_matching(&[])
                .filter(|arch| !ephemeral.intersects(arch.components()))
                .flat_map(|arch| arch.entities().to_vec())
                .collect()
        })
        .unwrap_or_default();

    despawn_entities(resources, &entities);
}

/// Removes `entities` from every store that knows about them.
fn despawn_entities(resources: &mut Resources, entities: &[crate::entity::Entity]) {
    for &entity in entities {
        if let Some(alloc) = resources.get_mut::<EntityAllocator>() {
            alloc.despawn(entity);
        }
        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
            archetypes.unregister_entity(entity);
        }
        if let Some(components) = resources.get_mut::<ComponentRegistry>() {
            components.remove_entity(entity);
        }
        if let Some(dynamic) = resources.get_mut::<DynamicComponents>() {
            dynamic.remove_entity(entity);
        }
    }

    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
        archetypes.gc_empty_archetypes();
    }
}

mod instances;

use super::{document, prefab};
use instances::*;
