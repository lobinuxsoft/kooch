//! [`DuplicateCommand`] — copies an entity and everything under it, beside the original.

use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;

use crate::actions::entity_state::{self, CapturedTree};
use crate::undo::EditorCommand;

pub(crate) struct DuplicateCommand {
    /// The subtree to build, read at construction time so the history is stable even if the source
    /// changes later.
    tree: CapturedTree,
    /// Where the source sat, so the copy is its sibling rather than a root.
    parent: Option<Entity>,
    /// What the last execute built, so undo knows what to take away.
    copies: Vec<Entity>,
}

impl DuplicateCommand {
    pub fn new(resources: &Resources, source: Entity) -> Self {
        Self {
            tree: entity_state::capture_tree(resources, source),
            parent: entity_state::parent_of(resources, source),
            copies: Vec::new(),
        }
    }
}

impl EditorCommand for DuplicateCommand {
    fn execute(&mut self, resources: &mut Resources) {
        self.copies.clear();
        self.copies = entity_state::paste_tree_local(resources, &self.tree, self.parent);
        // The copy belongs where the original does, or it lands under "Unsaved".
        if let Some(scene) = self
            .tree
            .first()
            .and_then(|captured| entity_state::scene_of(resources, captured.source))
        {
            for &entity in &self.copies {
                super::place::adopt(resources, entity, scene);
            }
        }
    }

    fn undo(&mut self, resources: &mut Resources) {
        for entity in self.copies.drain(..) {
            if let Some(alloc) = resources.get_mut::<EntityAllocator>() {
                alloc.despawn(entity);
            }
            if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                archetypes.unregister_entity(entity);
            }
            if let Some(components) = resources.get_mut::<ComponentRegistry>() {
                components.remove_entity(entity);
            }
        }
    }

    fn description(&self) -> &str {
        "Duplicate Entity"
    }
}
