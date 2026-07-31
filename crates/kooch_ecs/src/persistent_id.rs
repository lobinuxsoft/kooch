//! Stable entity identity that survives a save/load round trip.
//!
//! An [`Entity`] is a runtime handle: an index plus a generation, both
//! reassigned freely once the entity dies. That is the right shape for a
//! handle and the wrong shape for a file — a scene saved on Tuesday and
//! loaded on Wednesday hands out different indices, so anything that wrote
//! down an `Entity` now points somewhere else.
//!
//! Assets solved this already: `ReflectValue::AssetRef` addresses an asset
//! by [`Guid`](kooch_core::Guid), never by a live handle. [`PersistentId`] is
//! the same idea one level down.
//!
//! # Why ids are scene-local
//!
//! An [`EntityGuid`] is unique within its scene, not globally. That is what
//! lets the same scene be instantiated more than once — load a station
//! module twice and each copy remaps its ids independently, instead of both
//! copies claiming the same identity. Unity does this with
//! `SceneLoadFlags.NewInstance`, Unreal with Level Instances; a globally
//! unique id would make "entity X of scene Y" stop meaning anything the
//! moment scene Y is loaded twice.
//!
//! Cross-scene references carry the scene's own [`Guid`] alongside the
//! entity's id — see [`EntityRef`](crate::reflect::EntityRef).
//!
//! # Why the component is opt-in
//!
//! Only entities something actually points at carry a [`PersistentId`]. The
//! save path assigns one on demand when it writes a reference, so authors
//! never add it by hand. A procedurally generated galaxy should not pay
//! eight bytes and a map entry per entity that nothing references.

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
///
/// Present only on entities something points at — see the module docs for
/// why this is opt-in rather than universal.
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

/// Reflected so the id travels in a scene file as an ordinary component,
/// rather than as another special case beside `parent_index`.
///
/// Read-only in the inspector: the id is what every reference in the
/// scene resolves through, so editing it by hand would silently redirect
/// or orphan all of them.
impl Reflect for PersistentId {
    fn reflect_fields(&self) -> &'static [FieldMeta] {
        static FIELDS: &[FieldMeta] = &[FieldMeta {
            name: "id",
            type_name: "u64",
            kind: FieldKind::U64,
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
///
/// A counter rather than a random source, so that re-saving a scene
/// produces a clean diff instead of rewriting every id. The counter is
/// persisted with the scene: resetting it between sessions would reissue
/// ids that existing references already use.
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

    /// Resumes from a persisted watermark.
    ///
    /// Takes the larger of `next` and the current value: a scene file
    /// claiming a lower watermark than ids already handed out this session
    /// would reissue them, which is silent aliasing rather than a load
    /// error.
    pub fn resume_from(&mut self, next: u64) {
        self.next = self.next.max(next);
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
mod tests {
    use super::*;

    #[test]
    fn zero_is_not_a_valid_guid() {
        assert!(EntityGuid::new(0).is_none());
        assert!(EntityGuid::new(1).is_some());
    }

    /// The niche is the whole reason for `NonZeroU64`; if this regresses,
    /// every `Option<EntityGuid>` field silently doubles.
    #[test]
    fn an_optional_guid_costs_nothing_extra() {
        assert_eq!(
            size_of::<Option<EntityGuid>>(),
            size_of::<EntityGuid>(),
            "Option<EntityGuid> must stay 8 bytes",
        );
    }

    #[test]
    fn ids_are_sequential_and_never_zero() {
        let mut alloc = PersistentIdAllocator::new();
        let first = alloc.allocate();
        let second = alloc.allocate();
        assert_eq!(first.get(), 1);
        assert_eq!(second.get(), 2);
    }

    #[test]
    fn observing_an_id_stops_it_being_reissued() {
        let mut alloc = PersistentIdAllocator::new();
        alloc.observe(EntityGuid::new(42).unwrap());
        assert_eq!(alloc.allocate().get(), 43);
    }

    /// A scene file is not trusted to move the watermark backwards. Loading
    /// one that claims a lower value than ids already live would reissue
    /// them, and the aliasing would only show up as two entities answering
    /// to one reference.
    #[test]
    fn resuming_never_moves_the_watermark_backwards() {
        let mut alloc = PersistentIdAllocator::new();
        alloc.observe(EntityGuid::new(100).unwrap());
        alloc.resume_from(5);
        assert_eq!(alloc.allocate().get(), 101);
    }

    #[test]
    fn a_watermark_round_trips_through_a_fresh_allocator() {
        let mut alloc = PersistentIdAllocator::new();
        alloc.allocate();
        alloc.allocate();

        let mut reloaded = PersistentIdAllocator::new();
        reloaded.resume_from(alloc.watermark());
        assert_eq!(reloaded.allocate().get(), 3, "ids must not be reissued");
    }
}
