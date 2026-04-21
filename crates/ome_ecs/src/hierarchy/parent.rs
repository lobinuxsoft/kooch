//! Parent component — marks an entity as a child of another.

use crate::component::Component;
use crate::entity::Entity;
use crate::reflect::{
    FieldKind, FieldMeta, InspectorVisibility, Reflect, ReflectError, ReflectValue,
};

/// Marks this entity as a child of another entity.
///
/// `Parent` is the authoritative side of the relationship — the hierarchy
/// sync system updates `Children` to match.
#[derive(Debug, Clone)]
pub struct Parent {
    pub entity: Entity,
}

impl Component for Parent {}

impl Reflect for Parent {
    fn reflect_fields(&self) -> &'static [FieldMeta] {
        static FIELDS: &[FieldMeta] = &[FieldMeta {
            name: "entity",
            type_name: "Entity",
            kind: FieldKind::String,
            choices: &[],
        }];
        FIELDS
    }

    fn reflect_get(&self, field: &str) -> Option<ReflectValue> {
        match field {
            "entity" => Some(ReflectValue::String(format!(
                "{}:{}",
                self.entity.index(),
                self.entity.generation()
            ))),
            _ => None,
        }
    }

    fn reflect_set(&mut self, _field: &str, _value: ReflectValue) -> Result<(), ReflectError> {
        Err(ReflectError::ReadOnly)
    }

    fn reflect_default() -> Self {
        Self {
            entity: Entity::INVALID,
        }
    }

    fn inspector_visibility() -> InspectorVisibility {
        InspectorVisibility::ReadOnly
    }
}
