//! A reference from one component to an entity.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use kooch_core::Guid;

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
        /// The scene owning the target, or `None` for "the same scene as the reference itself".
        scene: Option<Guid>,
        /// Identity of the target within that scene.
        id: EntityGuid,
    },
}

/// The wire and on-disk shapes, told apart by which field is present.
#[derive(Serialize, Deserialize)]
struct Repr {
    /// The scene owning the target, for a persistent reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scene: Option<Guid>,
    /// Present on a persistent reference: what a file holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<EntityGuid>,
    /// Present on a live one: the index and generation the editor protocol
    /// carries, which mean something only to the session that issued them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    live: Option<(u32, u32)>,
}

/// Both states serialise, because the editor protocol is not a file.
impl Serialize for EntityRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match *self {
            Self::Persistent { scene, id } => Repr {
                scene,
                id: Some(id),
                live: None,
            },
            Self::Live(entity) => Repr {
                scene: None,
                id: None,
                live: Some((entity.index(), entity.generation())),
            },
        }
        .serialize(serializer)
    }
}

/// A reference read from a file is unresolved — the load pass turns it
/// into a [`EntityRef::Live`] once the target entity exists. One read off
/// the protocol is already live, and is taken as it came.
impl<'de> Deserialize<'de> for EntityRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let repr = Repr::deserialize(deserializer)?;
        match (repr.live, repr.id) {
            (Some((index, generation)), _) => Ok(Self::Live(Entity::new(index, generation))),
            (None, Some(id)) => Ok(Self::Persistent {
                scene: repr.scene,
                id,
            }),
            (None, None) => Err(serde::de::Error::custom(
                "an entity reference needs either `id` (persistent) or `live`",
            )),
        }
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
mod tests;
