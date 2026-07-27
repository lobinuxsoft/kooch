//! [`TransformEditCommand`] — captures the before/after `Transform` of
//! a single entity for one viewport gizmo drag (translate / rotate /
//! scale). One command per drag, not per frame: pushing a command on
//! every `TransformDelta` would flood the undo stack with ~60 entries
//! per second of dragging.

use ome_core::resource::Resources;
use ome_ecs::component::ComponentRegistry;
use ome_ecs::entity::Entity;
use ome_ecs::hierarchy::transform_propagation_system;
use ome_ecs::transform::Transform;

use crate::undo::EditorCommand;

pub(crate) struct TransformEditCommand {
    entity: Entity,
    before: Transform,
    after: Transform,
    desc: &'static str,
}

impl TransformEditCommand {
    pub fn new(entity: Entity, before: Transform, after: Transform, desc: &'static str) -> Self {
        Self {
            entity,
            before,
            after,
            desc,
        }
    }
}

impl EditorCommand for TransformEditCommand {
    fn execute(&mut self, resources: &mut Resources) {
        write_transform(resources, self.entity, self.after);
    }

    fn undo(&mut self, resources: &mut Resources) {
        write_transform(resources, self.entity, self.before);
    }

    fn description(&self) -> &str {
        self.desc
    }
}

fn write_transform(resources: &mut Resources, entity: Entity, transform: Transform) {
    let mut wrote = false;
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<Transform>()
        && let Some(t) = storage.get_mut(entity)
    {
        *t = transform;
        wrote = true;
    }
    if wrote {
        // Re-derive GlobalTransform so the same-frame renderer reads
        // the rolled-back pose without waiting for PostUpdate.
        transform_propagation_system(resources);
    }
}
