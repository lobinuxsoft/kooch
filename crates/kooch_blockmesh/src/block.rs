//! The component that puts a `BlockMesh` in a scene.

use kooch_core::Guid;
use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Binds an entity to the authoring mesh it is built from.
/// Feeds the renderer and physics rather than replacing them: [`sync_blocks`](crate::sync_blocks)
/// generates both from `source`.
#[derive(Debug, Clone, Default, Reflect)]
#[reflect(category = "Level")]
pub struct Block {
    /// The `.block` this block's geometry comes from. Generated meshes share its GUID, so entities
    /// with one source share one upload.
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
