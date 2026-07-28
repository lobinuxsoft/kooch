//! Add, remove and edit components the editor has no Rust type for.
//!
//! Their reflected counterparts key everything by `TypeId`, which a
//! plugin's types do not have here. These work by **name** instead,
//! against [`DynamicComponents`] — the same store the remote mirror
//! already uses, so the Inspector draws them with machinery that exists.
//!
//! Undo is snapshot-restore rather than a reverse operation: the field
//! values are already owned data, so keeping a copy is cheaper and more
//! honest than recomputing what a default used to be.

use ome_core::resource::Resources;
use ome_ecs::component::{DynamicType, DynamicTypeRegistry};
use ome_ecs::dynamic_components::DynamicComponents;
use ome_ecs::entity::Entity;
use ome_ecs::reflect::{FieldKind, ReflectValue};

use crate::undo::EditorCommand;

/// A field's value before anything has been authored into it.
///
/// The schema says what a field *is*, not what it starts as, so the
/// editor has to pick. Zero and empty are the only defensible answers —
/// anything else would be the editor inventing gameplay values.
pub(crate) fn default_value(kind: FieldKind) -> ReflectValue {
    match kind {
        FieldKind::F32 => ReflectValue::F32(0.0),
        FieldKind::F64 => ReflectValue::F64(0.0),
        FieldKind::U8 => ReflectValue::U8(0),
        FieldKind::U16 => ReflectValue::U16(0),
        FieldKind::U32 => ReflectValue::U32(0),
        FieldKind::U64 => ReflectValue::U64(0),
        FieldKind::I8 => ReflectValue::I8(0),
        FieldKind::I16 => ReflectValue::I16(0),
        FieldKind::I32 => ReflectValue::I32(0),
        FieldKind::I64 => ReflectValue::I64(0),
        FieldKind::Bool => ReflectValue::Bool(false),
        FieldKind::String => ReflectValue::String(String::new()),
        FieldKind::Vec2 => ReflectValue::Vec2(glam::Vec2::ZERO),
        FieldKind::Vec3 => ReflectValue::Vec3(glam::Vec3::ZERO),
        FieldKind::Vec4 => ReflectValue::Vec4(glam::Vec4::ZERO),
        FieldKind::Quat => ReflectValue::Quat(glam::Quat::IDENTITY),
        FieldKind::Mat4 => ReflectValue::Mat4(glam::Mat4::IDENTITY),
        FieldKind::AssetRef => ReflectValue::AssetRef {
            guid: None,
            asset_type: String::new(),
        },
        FieldKind::EntityRef => ReflectValue::EntityRef(None),
        // A nested struct has no flat value to stand in for it. Zero is
        // wrong and empty is wrong; the Inspector shows it read-only
        // until reflection can express nesting (#649).
        FieldKind::Nested => ReflectValue::String(String::new()),
    }
}

/// Every field of `ty`, at its starting value.
fn initial_fields(ty: &DynamicType) -> Vec<(String, ReflectValue)> {
    ty.fields
        .iter()
        .map(|f| (f.name.clone(), default_value(f.kind)))
        .collect()
}

/// Adds a plugin-declared component to an entity.
pub(crate) struct AddDynamicComponentCommand {
    entity: Entity,
    type_name: String,
    description: String,
}

impl AddDynamicComponentCommand {
    /// Fails when no such type is registered — a stale menu entry from a
    /// plugin that has since been unloaded.
    pub fn new(resources: &Resources, entity: Entity, type_name: &str) -> Option<Self> {
        resources.get::<DynamicTypeRegistry>()?.get(type_name)?;
        Some(Self {
            entity,
            type_name: type_name.to_owned(),
            description: format!("Add {}", short_name(type_name)),
        })
    }
}

impl EditorCommand for AddDynamicComponentCommand {
    fn execute(&mut self, resources: &mut Resources) {
        // The schema is read at execute time, not at construction: a
        // redo after a plugin reload should use the current shape of the
        // type, not the one captured when the menu item was clicked.
        let Some(fields) = resources
            .get::<DynamicTypeRegistry>()
            .and_then(|reg| reg.get(&self.type_name))
            .map(initial_fields)
        else {
            tracing::warn!(component = self.type_name, "type is no longer registered");
            return;
        };
        if let Some(store) = resources.get_mut::<DynamicComponents>() {
            store.insert(self.entity, &self.type_name, fields);
        }
    }

    fn undo(&mut self, resources: &mut Resources) {
        if let Some(store) = resources.get_mut::<DynamicComponents>() {
            store.remove(self.entity, &self.type_name);
        }
    }

    fn description(&self) -> &str {
        &self.description
    }
}

/// Removes a plugin-declared component, keeping its values for undo.
pub(crate) struct RemoveDynamicComponentCommand {
    entity: Entity,
    type_name: String,
    /// Values as they were, so undo restores the component rather than
    /// re-adding a blank one.
    snapshot: Option<Vec<(String, ReflectValue)>>,
    description: String,
}

impl RemoveDynamicComponentCommand {
    pub fn new(resources: &Resources, entity: Entity, type_name: &str) -> Self {
        let snapshot = resources.get::<DynamicComponents>().and_then(|store| {
            store
                .iter_entity(entity)
                .find(|(name, _)| *name == type_name)
                .map(|(_, fields)| fields.to_vec())
        });
        Self {
            entity,
            type_name: type_name.to_owned(),
            snapshot,
            description: format!("Remove {}", short_name(type_name)),
        }
    }
}

impl EditorCommand for RemoveDynamicComponentCommand {
    fn execute(&mut self, resources: &mut Resources) {
        if let Some(store) = resources.get_mut::<DynamicComponents>() {
            store.remove(self.entity, &self.type_name);
        }
    }

    fn undo(&mut self, resources: &mut Resources) {
        let Some(fields) = self.snapshot.clone() else {
            return;
        };
        if let Some(store) = resources.get_mut::<DynamicComponents>() {
            store.insert(self.entity, &self.type_name, fields);
        }
    }

    fn description(&self) -> &str {
        &self.description
    }
}

/// Edits one field of a plugin-declared component.
pub(crate) struct SetDynamicFieldCommand {
    entity: Entity,
    type_name: String,
    field: String,
    before: ReflectValue,
    after: ReflectValue,
    description: String,
}

impl SetDynamicFieldCommand {
    /// Fails when the component or field is absent, rather than writing
    /// a value nothing will read.
    pub fn new(
        resources: &Resources,
        entity: Entity,
        type_name: &str,
        field: String,
        value: ReflectValue,
    ) -> Option<Self> {
        let before = resources
            .get::<DynamicComponents>()?
            .iter_entity(entity)
            .find(|(name, _)| *name == type_name)?
            .1
            .iter()
            .find(|(name, _)| *name == field)?
            .1
            .clone();
        Some(Self {
            entity,
            type_name: type_name.to_owned(),
            description: format!("Set {}.{field}", short_name(type_name)),
            field,
            before,
            after: value,
        })
    }
}

impl EditorCommand for SetDynamicFieldCommand {
    fn execute(&mut self, resources: &mut Resources) {
        self.write(resources, self.after.clone());
    }

    fn undo(&mut self, resources: &mut Resources) {
        self.write(resources, self.before.clone());
    }

    fn description(&self) -> &str {
        &self.description
    }
}

impl SetDynamicFieldCommand {
    fn write(&self, resources: &mut Resources, value: ReflectValue) {
        if let Some(store) = resources.get_mut::<DynamicComponents>() {
            store.set_field(self.entity, &self.type_name, &self.field, value);
        }
    }
}

/// The last path segment, for a description that fits in a menu.
fn short_name(type_name: &str) -> &str {
    type_name.rsplit("::").next().unwrap_or(type_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ome_ecs::allocator::EntityAllocator;
    use ome_ecs::component::DynamicField;

    fn world() -> (Resources, Entity) {
        let mut resources = Resources::new();
        let mut alloc = EntityAllocator::new();
        let entity = alloc.spawn();
        resources.insert(alloc);
        resources.insert(DynamicComponents::new());

        let mut types = DynamicTypeRegistry::new();
        types
            .register(DynamicType {
                type_name: "my_game::Health".into(),
                fields: vec![
                    DynamicField {
                        name: "current".into(),
                        kind: FieldKind::U32,
                    },
                    DynamicField {
                        name: "regen".into(),
                        kind: FieldKind::F32,
                    },
                ],
                source: "my_game".into(),
            })
            .unwrap();
        resources.insert(types);
        (resources, entity)
    }

    fn fields_of(resources: &Resources, entity: Entity) -> Vec<(String, ReflectValue)> {
        resources
            .get::<DynamicComponents>()
            .unwrap()
            .iter_entity(entity)
            .find(|(name, _)| *name == "my_game::Health")
            .map(|(_, f)| f.to_vec())
            .unwrap_or_default()
    }

    #[test]
    fn adding_gives_every_field_a_starting_value() {
        let (mut resources, entity) = world();
        let mut cmd = AddDynamicComponentCommand::new(&resources, entity, "my_game::Health")
            .expect("type is registered");

        cmd.execute(&mut resources);

        let fields = fields_of(&resources, entity);
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0], ("current".into(), ReflectValue::U32(0)));
        assert_eq!(fields[1], ("regen".into(), ReflectValue::F32(0.0)));
    }

    #[test]
    fn undo_removes_what_add_created() {
        let (mut resources, entity) = world();
        let mut cmd =
            AddDynamicComponentCommand::new(&resources, entity, "my_game::Health").unwrap();

        cmd.execute(&mut resources);
        cmd.undo(&mut resources);

        assert!(fields_of(&resources, entity).is_empty());
    }

    /// A menu entry left over from a plugin that has been unloaded must
    /// not produce a command that writes a component with no schema.
    #[test]
    fn an_unregistered_type_yields_no_command() {
        let (resources, entity) = world();
        assert!(AddDynamicComponentCommand::new(&resources, entity, "gone::Type").is_none());
    }

    #[test]
    fn editing_a_field_round_trips_through_undo() {
        let (mut resources, entity) = world();
        AddDynamicComponentCommand::new(&resources, entity, "my_game::Health")
            .unwrap()
            .execute(&mut resources);

        let mut cmd = SetDynamicFieldCommand::new(
            &resources,
            entity,
            "my_game::Health",
            "current".into(),
            ReflectValue::U32(75),
        )
        .expect("component and field exist");

        cmd.execute(&mut resources);
        assert_eq!(fields_of(&resources, entity)[0].1, ReflectValue::U32(75));

        cmd.undo(&mut resources);
        assert_eq!(fields_of(&resources, entity)[0].1, ReflectValue::U32(0));
    }

    #[test]
    fn editing_an_absent_field_yields_no_command() {
        let (mut resources, entity) = world();
        AddDynamicComponentCommand::new(&resources, entity, "my_game::Health")
            .unwrap()
            .execute(&mut resources);

        assert!(
            SetDynamicFieldCommand::new(
                &resources,
                entity,
                "my_game::Health",
                "nonexistent".into(),
                ReflectValue::U32(1),
            )
            .is_none()
        );
        let _ = &mut resources;
    }

    /// Removing keeps the authored values, so undo restores the
    /// component rather than a blank one.
    #[test]
    fn removing_and_undoing_restores_the_values() {
        let (mut resources, entity) = world();
        AddDynamicComponentCommand::new(&resources, entity, "my_game::Health")
            .unwrap()
            .execute(&mut resources);
        SetDynamicFieldCommand::new(
            &resources,
            entity,
            "my_game::Health",
            "current".into(),
            ReflectValue::U32(42),
        )
        .unwrap()
        .execute(&mut resources);

        let mut cmd = RemoveDynamicComponentCommand::new(&resources, entity, "my_game::Health");
        cmd.execute(&mut resources);
        assert!(fields_of(&resources, entity).is_empty());

        cmd.undo(&mut resources);
        assert_eq!(
            fields_of(&resources, entity)[0].1,
            ReflectValue::U32(42),
            "undo restored a blank component instead of the authored one"
        );
    }

    /// Redo after a plugin reload must use the type's current shape.
    #[test]
    fn redo_reads_the_schema_again() {
        let (mut resources, entity) = world();
        let mut cmd =
            AddDynamicComponentCommand::new(&resources, entity, "my_game::Health").unwrap();
        cmd.execute(&mut resources);
        cmd.undo(&mut resources);

        // The plugin was rebuilt with an extra field.
        let mut types = resources.remove::<DynamicTypeRegistry>().unwrap();
        types
            .register(DynamicType {
                type_name: "my_game::Health".into(),
                fields: vec![DynamicField {
                    name: "shield".into(),
                    kind: FieldKind::Bool,
                }],
                source: "my_game".into(),
            })
            .unwrap();
        resources.insert(types);

        cmd.execute(&mut resources);

        let fields = fields_of(&resources, entity);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0], ("shield".into(), ReflectValue::Bool(false)));
    }
}
