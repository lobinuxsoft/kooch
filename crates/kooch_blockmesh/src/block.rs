//! The component that puts a `BlockMesh` in a scene.

use kooch_core::Guid;
use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Binds an entity to the authoring mesh it is built from.
///
/// The entity still carries a `MeshRenderer` and a `Collider` — this
/// does not replace them, it *feeds* them. [`sync_blocks`] generates the
/// render mesh and the collider from `source` and points both at the
/// result, which is the same division ProBuilder draws: the authoring
/// component owns the shape, and the ordinary renderer draws whatever
/// came out.
///
/// So a finished level needs no blocks at all. Bake the generated meshes
/// to assets, drop this component, and what ships is a scene of plain
/// mesh renderers.
///
/// [`sync_blocks`]: crate::sync_blocks
#[derive(Debug, Clone, Default, Reflect)]
#[reflect(category = "Level")]
pub struct Block {
    /// The `.blockmesh.ron` this block's geometry comes from.
    ///
    /// The generated render mesh and collider are published under this
    /// same GUID, so two entities sharing a source share one upload
    /// rather than paying for it twice.
    #[reflect(asset = "kooch_blockmesh::block_mesh::BlockMesh")]
    pub source: Option<Guid>,
}

impl Component for Block {}
