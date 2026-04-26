//! [`AddComponentCommand`] / [`RemoveComponentCommand`] — add or remove
//! a single reflected component from an entity, with snapshot-restore
//! on undo of remove.

use std::any::TypeId;

use ome_core::resource::Resources;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::component::ComponentRegistry;
use ome_ecs::entity::Entity;

use crate::undo::{ComponentSnapshot, EditorCommand};

pub(crate) struct AddComponentCommand {
    entity: Entity,
    type_id: TypeId,
}

impl AddComponentCommand {
    pub fn new(entity: Entity, type_id: TypeId) -> Self {
        Self { entity, type_id }
    }
}

impl EditorCommand for AddComponentCommand {
    fn execute(&mut self, resources: &mut Resources) {
        let mut inserted = false;
        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
            inserted = registry.insert_default_reflected(&self.type_id, self.entity);
        }
        if inserted {
            if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                if let Some(current) = archetypes.entity_archetype(self.entity) {
                    let new_arch =
                        archetypes.archetype_after_add_dynamic(current, self.type_id);
                    archetypes.register_entity(self.entity, new_arch);
                }
            }
        }
    }

    fn undo(&mut self, resources: &mut Resources) {
        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
            registry.remove_component(self.entity, &self.type_id);
        }
        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
            if let Some(current) = archetypes.entity_archetype(self.entity) {
                let new_arch =
                    archetypes.archetype_after_remove_dynamic(current, self.type_id);
                archetypes.register_entity(self.entity, new_arch);
            }
        }
    }

    fn description(&self) -> &str {
        "Add Component"
    }
}

pub(crate) struct RemoveComponentCommand {
    entity: Entity,
    type_id: TypeId,
    /// Snapshot of the removed component's reflected fields.
    snapshot: Option<ComponentSnapshot>,
}

impl RemoveComponentCommand {
    /// Creates the command, snapshotting the component's state before removal.
    pub fn new(resources: &Resources, entity: Entity, type_id: TypeId) -> Self {
        let snapshot = resources
            .get::<ComponentRegistry>()
            .and_then(|reg| reg.reflect_get_fields(&type_id, entity))
            .map(|fields| ComponentSnapshot { type_id, fields });

        Self {
            entity,
            type_id,
            snapshot,
        }
    }
}

impl EditorCommand for RemoveComponentCommand {
    fn execute(&mut self, resources: &mut Resources) {
        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
            registry.remove_component(self.entity, &self.type_id);
        }
        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
            if let Some(current) = archetypes.entity_archetype(self.entity) {
                let new_arch =
                    archetypes.archetype_after_remove_dynamic(current, self.type_id);
                archetypes.register_entity(self.entity, new_arch);
            }
        }
    }

    fn undo(&mut self, resources: &mut Resources) {
        // Re-insert the default component.
        let mut inserted = false;
        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
            inserted = registry.insert_default_reflected(&self.type_id, self.entity);
        }
        if inserted {
            if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                if let Some(current) = archetypes.entity_archetype(self.entity) {
                    let new_arch =
                        archetypes.archetype_after_add_dynamic(current, self.type_id);
                    archetypes.register_entity(self.entity, new_arch);
                }
            }
        }

        // Restore field values from snapshot.
        if let Some(ref snapshot) = self.snapshot {
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
        "Remove Component"
    }
}
