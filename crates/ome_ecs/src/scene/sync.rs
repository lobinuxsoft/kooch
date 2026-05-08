use crate::allocator::EntityAllocator;
use crate::archetype_registry::ArchetypeRegistry;
use crate::commands::Commands;
use crate::component::ComponentRegistry;
use ome_core::resource::Resources;

use super::document::SceneDocument;
use super::error::SceneError;

/// Clears the live ECS and rebuilds it from a [`SceneDocument`].
///
/// All existing entities are despawned. For each entity in the document,
/// a new entity is spawned, and its components are inserted via reflection.
/// Hierarchy relationships are reconstructed from `EntityDescription::parent`.
pub fn sync_scene_to_ecs(
    scene: &SceneDocument,
    resources: &mut Resources,
) -> Result<(), SceneError> {
    use crate::hierarchy::Parent;

    // 1. Despawn all existing entities.
    despawn_all(resources);

    // 2. First pass: spawn entities and insert components.
    // Track name → Entity for parent resolution.
    let mut name_to_entity: std::collections::HashMap<String, crate::entity::Entity> =
        std::collections::HashMap::new();
    let mut spawned_order: Vec<(crate::entity::Entity, Option<String>)> = Vec::new();

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
        spawned_order.push((entity, entity_desc.parent.clone()));

        for comp_desc in &entity_desc.components {
            // Look up the TypeId by full type name.
            let type_id = {
                let components = resources
                    .get::<ComponentRegistry>()
                    .ok_or_else(|| {
                        SceneError::UnknownComponent(comp_desc.type_name.clone())
                    })?;
                components
                    .type_id_by_name(&comp_desc.type_name)
                    .ok_or_else(|| {
                        SceneError::UnknownComponent(comp_desc.type_name.clone())
                    })?
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
                            let new_arch =
                                archetypes.archetype_after_add_dynamic(current, type_id);
                            archetypes.register_entity(entity, new_arch);
                        }
                    }
                }
            }

            // Set each field value.
            for (field_name, value) in &comp_desc.fields {
                if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                    registry.reflect_set_field(
                        &type_id,
                        entity,
                        field_name,
                        value.clone(),
                    )?;
                }
            }
        }
    }

    // 3. Second pass: establish hierarchy from parent names.
    let parent_tid = std::any::TypeId::of::<Parent>();
    for (entity, parent_name) in &spawned_order {
        if let Some(parent_name) = parent_name {
            if let Some(&parent_entity) = name_to_entity.get(parent_name) {
                if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                    registry.register_cpu_reflected::<Parent>();
                    if let Some(storage) = registry.get_cpu_mut::<Parent>() {
                        storage.insert(*entity, Parent { entity: parent_entity });
                    }
                }
                // Update the archetype to include Parent.
                if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                    if let Some(current) = archetypes.entity_archetype(*entity) {
                        let new_arch =
                            archetypes.archetype_after_add_dynamic(current, parent_tid);
                        archetypes.register_entity(*entity, new_arch);
                    }
                }
            }
        }
    }

    Ok(())
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

    for entity in entities {
        if let Some(alloc) = resources.get_mut::<EntityAllocator>() {
            alloc.despawn(entity);
        }
        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
            archetypes.unregister_entity(entity);
        }
        if let Some(components) = resources.get_mut::<ComponentRegistry>() {
            components.remove_entity(entity);
        }
    }

    // GC empty archetypes after clearing everything.
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
        archetypes.gc_empty_archetypes();
    }
}
