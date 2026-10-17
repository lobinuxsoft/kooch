//! [`BlockEditCommand`] — one drag's worth of reshaping, undone whole.

use kooch_blockmesh::BlockMesh;
use kooch_core::resource::Resources;
use kooch_ecs::entity::Entity;

use crate::undo::EditorCommand;

/// Restores a block's shape to what it was before an edit, or to what it became.
pub(crate) struct BlockEditCommand {
    entity: Entity,
    before: BlockMesh,
    after: BlockMesh,
}

impl BlockEditCommand {
    pub fn new(entity: Entity, before: BlockMesh, after: BlockMesh) -> Self {
        Self {
            entity,
            before,
            after,
        }
    }

    /// Puts one shape back on the component everything else is derived from.
    fn restore(&self, resources: &mut Resources, shape: &BlockMesh) {
        if crate::block_edit::set_shape(resources, self.entity, shape) {
            crate::block_edit::touch(resources, self.entity);
        }
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
