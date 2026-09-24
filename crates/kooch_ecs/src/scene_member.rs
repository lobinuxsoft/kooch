//! Which scene authored an entity.

use std::str::FromStr;

use kooch_core::Guid;

use crate::component::Component;
use crate::reflect::{
    FieldKind, FieldMeta, InspectorVisibility, Reflect, ReflectError, ReflectValue,
};

/// Marks the scene an entity was authored in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneMember {
    pub scene: Guid,
}

impl SceneMember {
    pub const fn new(scene: Guid) -> Self {
        Self { scene }
    }
}

impl Component for SceneMember {}

/// The membership travels as text.
static FIELDS: &[FieldMeta] = &[FieldMeta {
    name: "scene",
    // The kind says how it travels; this says what it is.
    type_name: "kooch_core::Guid",
    kind: FieldKind::String,
    choices: &[],
    bits: &[],
    range: None,
    shown_when: None,
    asset_type: "",
    requires: "",
    doc: "The scene this entity was authored in.",
    group: "",
    fields: &[],
    layers: false,
    layer: false,
}];

/// Reflected so a generic pass that rebuilds a world — `WorldSnapshot`, which stop restores from
/// — carries the membership across instead of dropping it.
impl Reflect for SceneMember {
    fn reflect_fields(&self) -> &'static [FieldMeta] {
        FIELDS
    }

    fn reflect_get(&self, field: &str) -> Option<ReflectValue> {
        match field {
            "scene" => Some(ReflectValue::String(self.scene.to_string())),
            _ => None,
        }
    }

    fn reflect_set(&mut self, field: &str, value: ReflectValue) -> Result<(), ReflectError> {
        match (field, value) {
            ("scene", ReflectValue::String(text)) => {
                self.scene = Guid::from_str(&text).map_err(|_| ReflectError::InvalidValue {
                    field: field.to_string(),
                    expected: "kooch_core::Guid",
                })?;
                Ok(())
            }
            ("scene", other) => Err(ReflectError::TypeMismatch {
                field: field.to_string(),
                expected: FieldKind::String,
                got: other.kind(),
            }),
            _ => Err(ReflectError::FieldNotFound(field.to_string())),
        }
    }

    fn reflect_default() -> Self {
        // Not a scene anybody has: a default-constructed membership is a value the restore is about
        // to overwrite, and if it ever leaks the entity reads as belonging to no scene rather than
        // to an arbitrary one.
        Self::new(Guid::from_bytes([0; 16]))
    }

    fn inspector_visibility() -> InspectorVisibility {
        // Derived, not authored — the World panel already says which
        // scene a row is in, and an editable copy could contradict it.
        InspectorVisibility::Hidden
    }
}

#[cfg(test)]
mod tests;
