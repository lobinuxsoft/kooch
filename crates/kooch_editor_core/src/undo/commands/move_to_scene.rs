//! [`MoveToSceneCommand`] — re-homes an entity into another open scene.
//!
//! 🔴 An entity's scene is a component, not a folder it sits in, which
//! is why this is a command at all: the drag that asks for it has to
//! land in the same history as every other edit, and undo has to know
//! where the entity was — including the case where it was nowhere.

use kooch_core::resource::Resources;
use kooch_ecs::entity::Entity;

use crate::undo::EditorCommand;

pub(crate) struct MoveToSceneCommand {
    entity: Entity,
    /// Where it is going.
    scene: kooch_core::Guid,
    /// Where it was, captured on the first execute. `None` means it
    /// belonged to no scene — the "Unsaved" group — and undo has to put
    /// it back there rather than into some other file.
    before: Option<kooch_core::Guid>,
    /// Whether `before` has been read yet, so a redo does not overwrite
    /// it with the destination this command itself wrote.
    captured: bool,
}

impl MoveToSceneCommand {
    pub fn new(entity: Entity, scene: kooch_core::Guid) -> Self {
        Self {
            entity,
            scene,
            before: None,
            captured: false,
        }
    }
}

impl EditorCommand for MoveToSceneCommand {
    fn execute(&mut self, resources: &mut Resources) {
        if !self.captured {
            self.before = super::place::scene_of(resources, self.entity);
            self.captured = true;
        }
        // The scene it LEFT is dirty too: its file no longer describes
        // what it holds. Marking only the destination would leave the
        // source looking saved with an entity missing from it.
        if let Some(before) = self.before
            && let Some(manager) = resources.get_mut::<kooch_ecs::SceneManager>()
        {
            manager.mark_scene_dirty(before);
        }
        super::place::adopt(resources, self.entity, self.scene);
    }

    fn undo(&mut self, resources: &mut Resources) {
        if let Some(manager) = resources.get_mut::<kooch_ecs::SceneManager>() {
            manager.mark_scene_dirty(self.scene);
        }
        match self.before {
            Some(before) => super::place::adopt(resources, self.entity, before),
            None => super::place::disown(resources, self.entity),
        }
    }

    fn description(&self) -> &str {
        "Move to Scene"
    }
}
