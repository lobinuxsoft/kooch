//! The component that puts a `BlockMesh` in a scene.

use kooch_core::Guid;
use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

use crate::BlockMesh;

/// A block's geometry, in the scene (#1452).
///
/// 🔴 **The authoring mesh is the component, not an asset.** ProBuilder does the same, and for the
/// same reason: a blockout's geometry belongs to one level, each instance has to be free to diverge
/// from the next, and the level is then one file to version and review. Turning a block into a
/// durable 3D object is a separate, deliberate export (#1069).
///
/// [`sync_blocks`](crate::sync_blocks) generates the render mesh and the collider from `mesh`.
#[derive(Debug, Clone, Default, Reflect)]
#[reflect(category = "Level")]
pub struct Block {
    /// The geometry itself, edited by the block tools.
    pub mesh: BlockMesh,
    /// ⚠️ **Legacy.** The `.block` this block's geometry used to come from, kept so a scene written
    /// before #1452 can still be read: the migration loads it once, copies it into `mesh` and
    /// clears this. A block that lost it before migrating is a block that loads empty with nothing
    /// failing, which is why it is not simply deleted.
    #[reflect(asset = "kooch_blockmesh::block_mesh::BlockMesh")]
    pub source: Option<Guid>,
}

impl Component for Block {}

/// Every component a block needs, by type id and by name.
/// 🔴 One list for both spawn paths — local by `TypeId`, remote by name — which had drifted twice
/// without failing. `Name` is absent: the wire's `spawn` creates it.
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
