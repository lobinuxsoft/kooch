//! Children component — ordered list of child entities.

use crate::component::Component;
use crate::entity::Entity;
use crate::reflect::{
    FieldKind, FieldMeta, InspectorVisibility, Reflect, ReflectError, ReflectValue,
};

/// Ordered list of child entities. Maintained automatically by the
/// hierarchy sync system based on `Parent` components.
#[derive(Debug, Clone, Default)]
pub struct Children {
    pub entities: Vec<Entity>,
}

impl Component for Children {}

impl Reflect for Children {
    fn reflect_fields(&self) -> &'static [FieldMeta] {
        static FIELDS: &[FieldMeta] = &[FieldMeta {
            name: "entities",
            type_name: "Vec<Entity>",
            kind: FieldKind::String,
            choices: &[],
            bits: &[],
            shown_when: None,
            asset_type: "",
            requires: "",
        }];
        FIELDS
    }

    fn reflect_get(&self, field: &str) -> Option<ReflectValue> {
        match field {
            "entities" => {
                let list: Vec<String> = self
                    .entities
                    .iter()
                    .map(|e| format!("{}:{}", e.index(), e.generation()))
                    .collect();
                Some(ReflectValue::String(list.join(", ")))
            }
            _ => None,
        }
    }

    fn reflect_set(&mut self, _field: &str, _value: ReflectValue) -> Result<(), ReflectError> {
        Err(ReflectError::ReadOnly)
    }

    fn reflect_default() -> Self {
        Self::default()
    }

    fn inspector_visibility() -> InspectorVisibility {
        InspectorVisibility::ReadOnly
    }
}
