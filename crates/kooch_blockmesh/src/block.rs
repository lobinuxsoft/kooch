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

/// Every component a block needs, by type id and by name.
///
/// 🔴 One list, two spawn paths. A block is created locally by an undo
/// command and remotely over the wire, and the two had already drifted
/// twice: the wire shipped without `PhysicsBody`, so physics never saw
/// the block at all, and the local one set a field on a component it
/// had forgotten to add. Neither failed — an absent component reads as
/// a default, and a write to one is a `None` nobody looks at.
///
/// Pairs rather than one or the other because the local path addresses
/// components by [`TypeId`] and the wire addresses them by name, and
/// keeping two lists in step by hand is what this replaces.
///
/// `Name` is not here: the wire's `spawn` call creates it, and the
/// local path resolves it through the registry with `Transform`.
pub fn block_components() -> [(std::any::TypeId, &'static str); 4] {
    [
        (
            std::any::TypeId::of::<Block>(),
            std::any::type_name::<Block>(),
        ),
        (
            std::any::TypeId::of::<kooch_ecs::mesh_renderer::MeshRenderer>(),
            std::any::type_name::<kooch_ecs::mesh_renderer::MeshRenderer>(),
        ),
        (
            std::any::TypeId::of::<kooch_physics::components::Collider>(),
            std::any::type_name::<kooch_physics::components::Collider>(),
        ),
        // Physics walks bodies, not colliders. Without one a block has
        // a shape nothing ever asks for.
        (
            std::any::TypeId::of::<kooch_physics::components::PhysicsBody>(),
            std::any::type_name::<kooch_physics::components::PhysicsBody>(),
        ),
    ]
}
