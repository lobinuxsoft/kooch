//! What a mesh-derived collider is addressed by.

use kooch_core::Guid;
use kooch_ecs::entity::Entity;

/// Where a mesh collider's geometry comes from: a shared asset GUID, or an entity's own generated
/// mesh. A `.block` named as an asset had two walks feeding it to the glTF parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MeshKey {
    /// A file something outside physics loaded and published.
    Asset(Guid),
    /// Geometry belonging to one entity, no file — per entity, since two blocks from one source
    /// differ once scaled.
    Owned(Entity),
}

impl From<Guid> for MeshKey {
    fn from(guid: Guid) -> Self {
        Self::Asset(guid)
    }
}

impl From<Entity> for MeshKey {
    fn from(entity: Entity) -> Self {
        Self::Owned(entity)
    }
}

#[cfg(test)]
mod tests;
