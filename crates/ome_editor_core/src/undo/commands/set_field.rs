//! [`SetFieldCommand`] — set a single reflected field on a component.

use std::any::TypeId;

use ome_core::resource::Resources;
use ome_ecs::component::ComponentRegistry;
use ome_ecs::entity::Entity;
use ome_ecs::hierarchy::transform_propagation_system;
use ome_ecs::reflect::ReflectValue;
use ome_ecs::transform::Transform;

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
        apply_field(resources, &self.type_id, self.entity, &self.field, &self.new_value);
    }

    fn undo(&mut self, resources: &mut Resources) {
        apply_field(resources, &self.type_id, self.entity, &self.field, &self.old_value);
    }

    fn description(&self) -> &str {
        "Set Field"
    }
}

/// Write a single reflected field, then re-derive `GlobalTransform`
/// when the mutation touched a `Transform` component. The Inspector
/// path used to skip propagation, so any same-frame consumer that
/// reads `GlobalTransform` (the raymarch BVH kick included) saw the
/// pre-edit pose for one extra frame. `TransformEditCommand` already
/// runs propagation for the gizmo path; this aligns the Inspector
/// with that contract — see #356.
fn apply_field(
    resources: &mut Resources,
    type_id: &TypeId,
    entity: Entity,
    field: &str,
    value: &ReflectValue,
) {
    let mut wrote = false;
    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        match registry.reflect_set_field(type_id, entity, field, value.clone()) {
            Ok(()) => wrote = true,
            Err(e) => tracing::warn!("undo: failed to set field '{field}': {e}"),
        }
    }
    if wrote && *type_id == TypeId::of::<Transform>() {
        transform_propagation_system(resources);
    }
}
