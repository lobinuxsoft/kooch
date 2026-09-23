//! [`PasteCommand`] — builds entities out of the clipboard.

use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;

use crate::actions::entity_state::{self, CapturedTree};
use crate::undo::EditorCommand;

pub(crate) struct PasteCommand {
    /// What to build, captured at copy time.
    states: Vec<CapturedTree>,
    /// What the last execute built, so undo knows what to take away.
    pasted: Vec<Entity>,
    /// Which scene the copies land in.
    into: crate::actions::SpawnTarget,
}

impl PasteCommand {
    /// `None` for an empty clipboard: a command that does nothing still
    /// takes a slot in the history, and undoing it would look broken.
    pub fn new(resources: &Resources, into: crate::actions::SpawnTarget) -> Option<Self> {
        let states = resources
            .get::<crate::clipboard::EntityClipboard>()?
            .states()
            .to_vec();
        match states.is_empty() {
            true => None,
            false => Some(Self {
                states,
                pasted: Vec::new(),
                into,
            }),
        }
    }
}

impl EditorCommand for PasteCommand {
    fn execute(&mut self, resources: &mut Resources) {
        self.pasted.clear();
        // 🔴 Resolved once for the whole paste, not once per entity. `SpawnTarget::NewScene` makes a
        // scene every time it is asked, so resolving it inside the loop would give a clipboard of
        // five entities five scenes holding one each.
        let scene = super::place::resolve_scene(resources, self.into);
        for tree in &self.states {
            let built = entity_state::paste_tree_local(resources, tree, None);
            for &entity in &built {
                // Without this the copy carries no `SceneMember`, lands under "Unsaved", and is
                // adopted by whichever scene happens to be active at the next save.
                if let Some(scene) = scene {
                    super::place::adopt(resources, entity, scene);
                }
            }
            self.pasted.extend(built);
        }
    }

    fn undo(&mut self, resources: &mut Resources) {
        for entity in self.pasted.drain(..) {
            if let Some(allocator) = resources.get_mut::<EntityAllocator>() {
                allocator.despawn(entity);
            }
            if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                archetypes.unregister_entity(entity);
            }
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                registry.remove_entity(entity);
            }
        }
    }

    fn description(&self) -> &str {
        "Paste"
    }
}
