//! What a mesh-derived collider is addressed by.

use kooch_core::Guid;
use kooch_ecs::entity::Entity;

/// Where the geometry of a mesh-derived collider comes from.
///
/// # Why two kinds
///
/// Most collision meshes are **assets**: a `.glb` on disk, loaded once,
/// shared by every entity that names it. One GUID, one shape, and a
/// hundred crates cost one upload.
///
/// A generated mesh is not that. It has no file, it belongs to the one
/// entity that authored it, and it changes while somebody drags it.
/// Addressing it by asset GUID meant naming a `.block` in a field that
/// means "a mesh on disk" — and every generic walk that resolves such a
/// field went and fed that file to a glTF parser. Two separate walks
/// grew the same guard to stop it, which is the field being wrong
/// rather than the walks.
///
/// So there are two ways to be addressed, and physics knows only that
/// much. It still does not know what a block is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MeshKey {
    /// A file something outside physics loaded and published.
    Asset(Guid),
    /// Geometry belonging to one entity, with no file behind it.
    ///
    /// Per entity rather than per source on purpose: two blocks built
    /// from one `.block` are two shapes the moment either is scaled,
    /// and sharing a cache entry between them is right only by luck.
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
