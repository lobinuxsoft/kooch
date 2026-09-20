//! Stable entity identity that survives a save/load round trip.

use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

use crate::component::Component;
use crate::reflect::{
    FieldKind, FieldMeta, InspectorVisibility, Reflect, ReflectError, ReflectValue,
};

/// Stable identity of an entity within its scene.
///
/// Non-zero so that `Option<EntityGuid>` is still eight bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EntityGuid(NonZeroU64);

impl EntityGuid {
    /// Wraps a raw id. `None` when `raw` is zero, which is reserved as the
    /// niche that keeps `Option<EntityGuid>` eight bytes wide.
    pub const fn new(raw: u64) -> Option<Self> {
        match NonZeroU64::new(raw) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }

    /// The underlying id.
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

impl std::fmt::Display for EntityGuid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.get())
    }
}

/// Marks an entity as referenceable across a save/load boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistentId {
    pub id: EntityGuid,
}

impl PersistentId {
    pub const fn new(id: EntityGuid) -> Self {
        Self { id }
    }
}

impl Component for PersistentId {}

/// Reflected so the id travels in a scene file as an ordinary component, rather than as another
/// special case beside `parent_index`.
impl Reflect for PersistentId {
    fn reflect_fields(&self) -> &'static [FieldMeta] {
        static FIELDS: &[FieldMeta] = &[FieldMeta {
            name: "id",
            group: "",
            doc: "Stable identity that survives saving, loading and reordering.\n\nAssigned once \
by the engine. Two entities with the same id are one entity as far as \
every reference in the project is concerned.",
            type_name: "u64",
            kind: FieldKind::U64,
            choices: &[],
            bits: &[],
            range: None,
            shown_when: None,
            asset_type: "",
            requires: "",
            fields: &[],
            layers: false,
        }];
        FIELDS
    }

    fn reflect_get(&self, field: &str) -> Option<ReflectValue> {
        match field {
            "id" => Some(ReflectValue::U64(self.id.get())),
            _ => None,
        }
    }

    fn reflect_set(&mut self, field: &str, value: ReflectValue) -> Result<(), ReflectError> {
        match field {
            "id" => match value {
                ReflectValue::U64(raw) => {
                    // Zero is the niche, not an id. A file carrying one is
                    // corrupt, and accepting it would make the entity
                    // unreferenceable in a way nothing later could explain.
                    self.id = EntityGuid::new(raw).ok_or(ReflectError::TypeMismatch {
                        field: "id".into(),
                        expected: FieldKind::U64,
                        got: FieldKind::U64,
                    })?;
                    Ok(())
                }
                other => Err(ReflectError::TypeMismatch {
                    field: "id".into(),
                    expected: FieldKind::U64,
                    got: other.kind(),
                }),
            },
            other => Err(ReflectError::FieldNotFound(other.into())),
        }
    }

    fn reflect_default() -> Self {
        // The allocator overwrites this immediately; it exists because
        // reflected insertion builds a default first.
        Self::new(EntityGuid::new(1).expect("non-zero"))
    }

    fn inspector_visibility() -> InspectorVisibility {
        InspectorVisibility::ReadOnly
    }
}

/// Hands out [`EntityGuid`]s for one scene.
#[derive(Debug, Clone)]
pub struct PersistentIdAllocator {
    next: u64,
}

impl Default for PersistentIdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl PersistentIdAllocator {
    /// Starts at 1 — zero is the [`EntityGuid`] niche.
    pub const fn new() -> Self {
        Self { next: 1 }
    }

    /// The value to persist so a later session does not reissue live ids.
    pub const fn watermark(&self) -> u64 {
        self.next
    }

    /// Allocates the next id.
    pub fn allocate(&mut self) -> EntityGuid {
        // A u64 counter incremented once per referenced entity does not
        // reach zero again in any run this engine will see, so the
        // `expect` documents an invariant rather than guarding a case.
        let id = EntityGuid::new(self.next).expect("allocator never yields zero");
        self.next += 1;
        id
    }

    /// Notes that `id` is in use, so it is never handed out again.
    ///
    /// Called when loading a scene whose entities already carry ids.
    pub fn observe(&mut self, id: EntityGuid) {
        self.next = self.next.max(id.get() + 1);
    }
}

#[cfg(test)]
mod tests;
