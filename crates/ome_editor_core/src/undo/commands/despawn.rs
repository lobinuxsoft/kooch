//! [`DespawnCommand`] — despawns an entity, snapshotting its reflected
//! component state so undo can revive it with the same handle.

use std::any::TypeId;
use std::collections::BTreeSet;

use ome_core::resource::Resources;
use ome_ecs::allocator::EntityAllocator;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::component::ComponentRegistry;
use ome_ecs::entity::Entity;

use crate::undo::{ComponentSnapshot, EditorCommand};

pub(crate) struct DespawnCommand {
    entity: Entity,
    /// Snapshots of all reflected components, captured before despawn.
    snapshots: Vec<ComponentSnapshot>,
    /// All component TypeIds the entity had (including non-reflected).
    component_types: BTreeSet<TypeId>,
}

impl DespawnCommand {
    /// Creates the command, snapshotting the entity's reflected component state.
    pub fn new(resources: &Resources, entity: Entity) -> Self {
        let mut snapshots = Vec::new();
        let mut component_types = BTreeSet::new();

        // Get the entity's archetype to know which components it has.
        if let Some(archetypes) = resources.get::<ArchetypeRegistry>() {
            if let Some(arch_id) = archetypes.entity_archetype(entity) {
                if let Some(arch) = archetypes.get(arch_id) {
                    component_types = arch.components().clone();
                }
            }
        }

        // Snapshot all reflected components.
        if let Some(registry) = resources.get::<ComponentRegistry>() {
            for &type_id in &component_types {
                if let Some(fields) = registry.reflect_get_fields(&type_id, entity) {
                    snapshots.push(ComponentSnapshot { type_id, fields });
                }
            }
        }

        Self {
            entity,
            snapshots,
            component_types,
        }
    }
}

impl EditorCommand for DespawnCommand {
    fn execute(&mut self, resources: &mut Resources) {
        if let Some(alloc) = resources.get_mut::<EntityAllocator>() {
            alloc.despawn(self.entity);
        }
        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
            archetypes.unregister_entity(self.entity);
        }
        if let Some(components) = resources.get_mut::<ComponentRegistry>() {
            components.remove_entity(self.entity);
        }
    }

    fn undo(&mut self, resources: &mut Resources) {
        // Revive the entity at its original slot.
        let revived = resources
            .get_mut::<EntityAllocator>()
            .is_some_and(|alloc| alloc.revive(self.entity));

        if !revived {
            tracing::warn!(
                "undo: failed to revive entity {} — slot may have been reused",
                self.entity
            );
            return;
        }

        // Re-register into EMPTY archetype first.
        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
            archetypes.register_entity(self.entity, ome_ecs::archetype::ArchetypeId::EMPTY);
        }

        // Restore all components from snapshots.
        for type_id in &self.component_types {
            // Insert default component.
            let mut inserted = false;
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                inserted = registry.insert_default_reflected(type_id, self.entity);
            }
            if inserted {
                // Transition archetype.
                if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                    if let Some(current) = archetypes.entity_archetype(self.entity) {
                        let new_arch = archetypes.archetype_after_add_dynamic(current, *type_id);
                        archetypes.register_entity(self.entity, new_arch);
                    }
                }
            }
        }

        // Restore reflected field values from snapshots.
        for snapshot in &self.snapshots {
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                for (field, value) in &snapshot.fields {
                    if let Err(e) = registry.reflect_set_field(
                        &snapshot.type_id,
                        self.entity,
                        field,
                        value.clone(),
                    ) {
                        tracing::warn!("undo: failed to restore field '{field}': {e}");
                    }
                }
            }
        }
    }

    fn description(&self) -> &str {
        "Despawn Entity"
    }
}
