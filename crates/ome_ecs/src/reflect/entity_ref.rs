//! A reference from one component to an entity.
//!
//! The same reference has to be two different things depending on where it
//! lives, and pretending otherwise is what forced `parent_index` to exist:
//!
//! - **In memory** a component needs an [`Entity`] — a query, a storage
//!   lookup and a despawn check all take one, and translating on every
//!   access would be absurd.
//! - **In a file** an [`Entity`] is meaningless. Indices and generations
//!   are reassigned on the next load, so a saved handle points at whatever
//!   happens to occupy that slot.
//!
//! [`EntityRef`] holds both states explicitly rather than picking one and
//! papering over the other. The conversion happens at exactly two places —
//! the scene save path and the load path's remapping pass — so the rest of
//! the engine only ever sees [`EntityRef::Live`].

use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::Error as _};

use ome_core::Guid;

use crate::entity::Entity;
use crate::persistent_id::EntityGuid;

/// A reference to an entity, in whichever form its current home requires.
///
/// See the module docs for why this is two states and not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityRef {
    /// A live handle. What a running component holds, and what the
    /// inspector shows.
    Live(Entity),
    /// A persistent reference. What a scene file holds.
    Persistent {
        /// The scene owning the target, or `None` for "the same scene as
        /// the reference itself".
        ///
        /// `None` is not merely a shorthand: it keeps a scene relocatable.
        /// A scene that spelled out its own [`Guid`] in every internal
        /// reference could not be copied into a new scene without
        /// rewriting all of them.
        scene: Option<Guid>,
        /// Identity of the target within that scene.
        id: EntityGuid,
    },
}

/// The on-disk shape. Only [`EntityRef::Persistent`] has one.
#[derive(Serialize, Deserialize)]
struct PersistentRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scene: Option<Guid>,
    id: EntityGuid,
}

/// Serialising a [`EntityRef::Live`] fails loudly rather than writing a
/// runtime handle into a file.
///
/// [`Entity`] deliberately does not implement [`Serialize`] — an index and
/// a generation mean nothing once reloaded. A `Live` reaching a serialiser
/// means the save path failed to resolve it, and the difference between an
/// error here and a silently written handle is the difference between a
/// failed save and a scene that loads with references pointing at
/// arbitrary entities.
impl Serialize for EntityRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match *self {
            Self::Persistent { scene, id } => PersistentRef { scene, id }.serialize(serializer),
            Self::Live(entity) => Err(S::Error::custom(format!(
                "cannot serialise a live entity reference ({}:{}); \
                 the save path must resolve it to a PersistentId first",
                entity.index(),
                entity.generation(),
            ))),
        }
    }
}

/// A reference read from a file is always unresolved — the load pass turns
/// it into a [`EntityRef::Live`] once the target entity exists.
impl<'de> Deserialize<'de> for EntityRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let PersistentRef { scene, id } = PersistentRef::deserialize(deserializer)?;
        Ok(Self::Persistent { scene, id })
    }
}

impl EntityRef {
    /// A reference to a live entity.
    pub const fn live(entity: Entity) -> Self {
        Self::Live(entity)
    }

    /// A reference to an entity in the same scene.
    pub const fn same_scene(id: EntityGuid) -> Self {
        Self::Persistent { scene: None, id }
    }

    /// A reference to an entity in another scene.
    pub const fn in_scene(scene: Guid, id: EntityGuid) -> Self {
        Self::Persistent {
            scene: Some(scene),
            id,
        }
    }

    /// The live handle, or `None` if this reference has not been resolved.
    ///
    /// A `None` here is the normal state for a reference into a scene that
    /// is not resident — under world-cell streaming that is an unresolved
    /// reference, not a broken one.
    pub const fn entity(self) -> Option<Entity> {
        match self {
            Self::Live(entity) => Some(entity),
            Self::Persistent { .. } => None,
        }
    }

    /// The persistent identity, or `None` if this reference is still live.
    pub const fn persistent_id(self) -> Option<EntityGuid> {
        match self {
            Self::Persistent { id, .. } => Some(id),
            Self::Live(_) => None,
        }
    }

    /// The scene this reference points into, if it names one.
    pub const fn scene(self) -> Option<Guid> {
        match self {
            Self::Persistent { scene, .. } => scene,
            Self::Live(_) => None,
        }
    }

    /// Whether this reference still needs the load pass to resolve it.
    pub const fn is_unresolved(self) -> bool {
        matches!(self, Self::Persistent { .. })
    }
}

impl std::fmt::Display for EntityRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Live(entity) => write!(f, "{}:{}", entity.index(), entity.generation()),
            Self::Persistent { scene: None, id } => write!(f, "#{id}"),
            Self::Persistent {
                scene: Some(scene),
                id,
            } => write!(f, "{scene}#{id}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guid(raw: u64) -> EntityGuid {
        EntityGuid::new(raw).expect("non-zero")
    }

    #[test]
    fn a_live_reference_yields_its_entity_and_no_identity() {
        let entity = Entity::new(7, 2);
        let reference = EntityRef::live(entity);
        assert_eq!(reference.entity(), Some(entity));
        assert_eq!(reference.persistent_id(), None);
        assert!(!reference.is_unresolved());
    }

    #[test]
    fn a_persistent_reference_yields_its_identity_and_no_entity() {
        let reference = EntityRef::same_scene(guid(9));
        assert_eq!(reference.entity(), None);
        assert_eq!(reference.persistent_id(), Some(guid(9)));
        assert!(reference.is_unresolved());
    }

    /// An internal reference must not name its own scene, or copying the
    /// scene would carry references back to the original.
    #[test]
    fn a_same_scene_reference_names_no_scene() {
        assert_eq!(EntityRef::same_scene(guid(1)).scene(), None);
    }

    #[test]
    fn a_cross_scene_reference_keeps_its_scene() {
        let scene = Guid::new_v4();
        assert_eq!(EntityRef::in_scene(scene, guid(1)).scene(), Some(scene));
    }

    /// A same-scene reference must not serialise a `scene: None` field —
    /// every internal link in every scene file would carry it.
    #[test]
    fn a_same_scene_reference_omits_the_scene_field() {
        let encoded = ron::to_string(&EntityRef::same_scene(guid(3))).expect("serialises");
        assert!(
            !encoded.contains("scene"),
            "same-scene refs must stay terse, got {encoded}",
        );
    }

    /// Writing a runtime handle into a file is the exact bug this type
    /// exists to prevent, so it has to fail rather than round-trip.
    #[test]
    fn serialising_a_live_reference_is_an_error() {
        let result = ron::to_string(&EntityRef::live(Entity::new(3, 1)));
        let Err(error) = result else {
            panic!("a live reference must not serialise");
        };
        assert!(
            error.to_string().contains("resolve"),
            "the error should say what to do, got: {error}",
        );
    }

    #[test]
    fn a_persistent_reference_round_trips_through_ron() {
        let scene = Guid::new_v4();
        for original in [
            EntityRef::same_scene(guid(1)),
            EntityRef::in_scene(scene, guid(2)),
        ] {
            let encoded = ron::to_string(&original).expect("serialises");
            let decoded: EntityRef = ron::from_str(&encoded).expect("deserialises");
            assert_eq!(decoded, original);
        }
    }
}
