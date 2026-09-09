//! [`BlockEditCommand`] — one drag's worth of reshaping, undone whole.

use kooch_blockmesh::BlockMesh;
use kooch_core::Guid;
use kooch_core::resource::Resources;
use kooch_ecs::entity::Entity;

use crate::undo::EditorCommand;

/// Restores a block's shape to what it was before an edit, or to what
/// it became.
///
/// # Why the whole mesh
///
/// A rotate and a scale are not invertible by negating what the handle
/// reported, a drag that snapped is not the drag the mouse described,
/// and an **extrude changes the topology** — there are faces afterwards
/// that had no before, so putting positions back would leave them
/// indexing corners that are gone.
///
/// A block is eight corners and six faces. A level of them is still a
/// rounding error beside one mesh in the pool.
pub(crate) struct BlockEditCommand {
    entity: Entity,
    source: Guid,
    before: BlockMesh,
    after: BlockMesh,
}

impl BlockEditCommand {
    pub fn new(entity: Entity, source: Guid, before: BlockMesh, after: BlockMesh) -> Self {
        Self {
            entity,
            source,
            before,
            after,
        }
    }

    /// Puts one shape back and republishes everything derived from it.
    fn restore(&self, resources: &mut Resources, shape: &BlockMesh) {
        if !crate::block_edit::set_shape(resources, self.source, shape) {
            return;
        }
        // The same announcement a drag makes. Without it the collider
        // and the render mesh keep the shape they were built from, and
        // an undo moves the outline while the body stays where it was.
        crate::block_edit::announce(resources, self.source);
        crate::block_edit::save(resources, self.entity);
    }
}

impl EditorCommand for BlockEditCommand {
    fn execute(&mut self, resources: &mut Resources) {
        self.restore(resources, &self.after.clone());
    }

    fn undo(&mut self, resources: &mut Resources) {
        self.restore(resources, &self.before.clone());
    }

    fn description(&self) -> &str {
        "Edit Block"
    }
}
