//! [`SetFieldCommand`] — set a single reflected field on a component.

use std::any::TypeId;

use ome_core::resource::Resources;
use ome_ecs::component::ComponentRegistry;
use ome_ecs::entity::Entity;
use ome_ecs::reflect::ReflectValue;

use crate::undo::EditorCommand;

pub(crate) struct SetFieldCommand {
    entity: Entity,
    type_id: TypeId,
    field: String,
    new_value: ReflectValue,
    old_value: ReflectValue,
}

impl SetFieldCommand {
    /// Creates the command, capturing the old field value from the registry.
    ///
    /// Returns `None` if the component/field doesn't exist.
    pub fn new(
        resources: &Resources,
        entity: Entity,
        type_id: TypeId,
        field: String,
        new_value: ReflectValue,
    ) -> Option<Self> {
        let registry = resources.get::<ComponentRegistry>()?;
        let fields = registry.reflect_get_fields(&type_id, entity)?;
        let old_value = fields
            .into_iter()
            .find(|(name, _)| name == &field)
            .map(|(_, v)| v)?;
        Some(Self {
            entity,
            type_id,
            field,
            new_value,
            old_value,
        })
    }
}

impl EditorCommand for SetFieldCommand {
    fn execute(&mut self, resources: &mut Resources) {
        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
            if let Err(e) = registry.reflect_set_field(
                &self.type_id,
                self.entity,
                &self.field,
                self.new_value.clone(),
            ) {
                tracing::warn!("undo: failed to set field '{}': {e}", self.field);
            }
        }
    }

    fn undo(&mut self, resources: &mut Resources) {
        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
            if let Err(e) = registry.reflect_set_field(
                &self.type_id,
                self.entity,
                &self.field,
                self.old_value.clone(),
            ) {
                tracing::warn!("undo: failed to restore field '{}': {e}", self.field);
            }
        }
    }

    fn description(&self) -> &str {
        "Set Field"
    }
}
