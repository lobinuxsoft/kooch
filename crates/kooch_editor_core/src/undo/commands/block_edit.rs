//! [`BlockEditCommand`] — one drag's worth of reshaping, undone whole.

use glam::Vec3;
use kooch_core::Guid;
use kooch_core::resource::Resources;
use kooch_ecs::entity::Entity;

use crate::undo::EditorCommand;

/// Restores a block's corners to what they were before a drag, or to
/// what they became.
///
/// # Why the positions and not the delta
///
/// A rotate and a scale are not invertible by negating what the handle
/// reported, and a drag that snapped is not the drag the mouse
/// described. The corners are the truth, and a block has eight of them
/// — a level's worth of blocks is still a rounding error beside one
/// mesh in the pool.
pub(crate) struct BlockEditCommand {
    entity: Entity,
    source: Guid,
    before: Vec<Vec3>,
    after: Vec<Vec3>,
}

impl BlockEditCommand {
    pub fn new(entity: Entity, source: Guid, before: Vec<Vec3>, after: Vec<Vec3>) -> Self {
        Self {
            entity,
            source,
            before,
            after,
        }
    }

    /// Puts one set of corners back and republishes everything derived
    /// from them.
    fn restore(&self, resources: &mut Resources, corners: &[Vec3]) {
        if !crate::block_edit::set_corners(resources, self.source, corners) {
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
