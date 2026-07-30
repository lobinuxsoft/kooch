use crate::allocator::EntityAllocator;
use crate::archetype_registry::ArchetypeRegistry;
use crate::commands::Commands;
use crate::component::ComponentRegistry;
use crate::dynamic_components::DynamicComponents;
use ome_core::resource::Resources;

use super::document::SceneDocument;
use super::entity_refs::{DeferredRef, resolve_deferred};
use super::error::SceneError;

/// Clears the live ECS and rebuilds it from a [`SceneDocument`].
///
/// Every non-ephemeral entity is despawned first, so this is "open this
/// scene and only this scene". To add a scene beside the ones already
/// open, use [`spawn_scene_into`].
pub fn sync_scene_to_ecs(
    scene: &SceneDocument,
    resources: &mut Resources,
) -> Result<(), SceneError> {
    despawn_all(resources);
    spawn_scene_into(scene, resources)
}

/// Spawns a document's entities beside whatever is already loaded.
///
/// Each entity is tagged with [`SceneMember`] naming `scene.id`, which is
/// what lets saving write only its own entities and unloading despawn only
/// its own.
///
/// Entity references resolve at the end, once every entity in this
/// document exists. A reference into a scene that is not open stays
/// unresolved rather than failing — see
/// [`resolve_deferred`](super::entity_refs::resolve_deferred).
pub fn spawn_scene_into(
    scene: &SceneDocument,
    resources: &mut Resources,
) -> Result<(), SceneError> {
    spawn_returning(scene, resources).map(|_| ())
}

/// Stamps out a copy of `prefab` inside the scene `into`, and hands back
/// its root.
///
/// # Instancing is not opening
///
/// Opening a scene maps the file's entity ids one-to-one onto entities and
/// can be saved back to the file. Instancing remaps them, and the result
/// belongs to the scene that contains it. Only the first has a reason to
/// refuse a second copy — which is why #609's "already open" rule must not
/// reach here.
///
/// # What an instance keeps, and what it does not
///
/// The entities are baked into `into`: they carry no link back to the file
/// they came from, so editing the prefab afterwards does not update them
/// (#611 Phase B is where that link and its per-field overrides live). What
/// this does give is the operation a game actually needs — spawn a bullet,
/// a tree, an enemy — and none of it is thrown away by adding the link
/// later.
///
/// Fails with [`SceneError::NotASingleRoot`] when the document is not one
/// tree; see there for why a unit needs a single root.
pub fn instantiate(
    prefab: &SceneDocument,
    resources: &mut Resources,
    into: ome_core::Guid,
) -> Result<crate::entity::Entity, SceneError> {
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
    Ok(spawned[root])
}

/// Shared body of [`spawn_scene_into`] and [`instantiate`], handing back
/// the entities it spawned in document order.
fn spawn_returning(
    scene: &SceneDocument,
    resources: &mut Resources,
) -> Result<Vec<crate::entity::Entity>, SceneError> {
    use crate::hierarchy::Parent;

    // Identity has to be a known type before the spawn pass, or the ids
    // in the file get parked as an unknown component and every reference
    // resolves to nothing. Registering here rather than relying on
    // `EcsPlugin` keeps a hand-built `Resources` loading correctly.
    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.register_cpu_reflected::<crate::persistent_id::PersistentId>();
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
        // Spawn a fresh entity.
        let entity = {
            let mut commands = resources
                .remove::<Commands>()
                .expect("Commands not found in Resources");
            let entity = commands.spawn(resources).id();
            resources.insert(commands);
            entity
        };

        name_to_entity.insert(entity_desc.name.clone(), entity);
        spawned_order.push(entity);
        tag_with_scene(resources, entity, scene.id);

        for comp_desc in &entity_desc.components {
            // Look up the TypeId by full type name. A name this binary
            // has no type for is parked verbatim rather than failing the
            // load: which components resolve depends on which binary
            // opened the scene, and aborting here would despawn the
            // world (step 1 already ran) and lose everything on the next
            // save. See `DynamicComponents`.
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
                if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                    registry.reflect_set_field(&type_id, entity, field_name, value.clone())?;
                }
            }
        }
    }

    // 3. Second pass: rebuild the hierarchy of *legacy* scenes only.
    //
    // A scene written since #607 carries `Parent` as an ordinary component
    // whose entity reference the remapping pass below resolves, the same
    // way it resolves any other component pointing at an entity. Older
    // files put the link out of band, so they still need this.
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
                        target: "ome_ecs::scene",
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

    Ok(spawned_order)
}

/// Records which scene an entity was authored in.
fn tag_with_scene(resources: &mut Resources, entity: crate::entity::Entity, scene: ome_core::Guid) {
    use crate::scene_member::SceneMember;

    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.register_cpu::<SceneMember>();
        if let Some(storage) = registry.get_cpu_mut::<SceneMember>() {
            storage.insert(entity, SceneMember::new(scene));
        }
    }
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
        && let Some(current) = archetypes.entity_archetype(entity)
    {
        let next =
            archetypes.archetype_after_add_dynamic(current, std::any::TypeId::of::<SceneMember>());
        archetypes.register_entity(entity, next);
    }
}

/// Despawns only the entities belonging to `scene`.
///
/// "Remove the station" and "I walked away" have to be different
/// operations (#566); this is the first of the two.
pub fn despawn_scene(scene: ome_core::Guid, resources: &mut Resources) {
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
///
/// Entities whose archetype contains a marker registered in
/// [`EphemeralComponents`](crate::ephemeral::EphemeralComponents) are
/// preserved across scene loads. This keeps editor helper entities
/// (cameras, gizmos) alive when the user opens a different scene.
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
///
/// Shared by the whole-world and per-scene paths so they cannot drift into
/// forgetting different stores.
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
