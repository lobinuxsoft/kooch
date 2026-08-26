//! Which scene authored an entity.
//!
//! With more than one scene open, "who owns this entity" stops being
//! implicit. Saving has to write only its own entities, unloading has to
//! despawn only its own, and the World panel has to say which file a row
//! came from.
//!
//! # Why this is not serialised
//!
//! Every entity in a file belongs to that file's scene, so writing the
//! membership into the file stores the same fact twice and lets the two
//! copies disagree — a file claiming an entity belongs to a scene it is not
//! in is a contradiction nothing could resolve. It is assigned on load
//! instead, the same way [`Children`](crate::hierarchy::Children) and
//! [`GlobalTransform`](crate::hierarchy::GlobalTransform) are derived
//! rather than stored.
//!
//! # Scene membership is not cell residency
//!
//! This is the *authoring* home — a human decision, stored. Which cell an
//! entity is in is derived from its transform and changes as it moves
//! (#566). They are orthogonal: a scene spans many cells, and several
//! scenes can overlap one cell. Storing residency here would go stale the
//! first time something moved.

use std::str::FromStr;

use kooch_core::Guid;

use crate::component::Component;
use crate::reflect::{
    FieldKind, FieldMeta, InspectorVisibility, Reflect, ReflectError, ReflectValue,
};

/// Marks the scene an entity was authored in.
///
/// Absent on entities that belong to no scene — editor cameras, gizmo
/// helpers, and anything else marked
/// [`ephemeral`](crate::ephemeral::EphemeralComponents).
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
///
/// A [`Guid`] has no [`FieldKind`] of its own, and giving it one would
/// drag every reflection consumer and the whole inspector along for a
/// component none of them draw. `Display` and `FromStr` round-trip a
/// `Guid` exactly, so `String` carries it without inventing a kind.
static FIELDS: &[FieldMeta] = &[FieldMeta {
    name: "scene",
    // The kind says how it travels; this says what it is.
    type_name: "kooch_core::Guid",
    kind: FieldKind::String,
    choices: &[],
    bits: &[],
    shown_when: None,
    asset_type: "",
    requires: "",
    doc: "The scene this entity was authored in.",
    group: "",
}];

/// Reflected so a generic pass that rebuilds a world — [`WorldSnapshot`],
/// which stop restores from — carries the membership across instead of
/// dropping it.
///
/// Being reflected is not the same as being saved: `SceneDocument`
/// excludes this type by `TypeId` when it writes a scene, so the file
/// still states membership once, by which file an entity is in.
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
        // Not a scene anybody has: a default-constructed membership is a
        // value the restore is about to overwrite, and if it ever leaks
        // the entity reads as belonging to no scene rather than to an
        // arbitrary one.
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
