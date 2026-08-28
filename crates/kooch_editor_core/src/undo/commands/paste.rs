//! [`PasteCommand`] — builds entities out of the clipboard.
//!
//! The local half of Ctrl+V. Remote mode has its own path, because the
//! entities have to be built by the process that owns the world; both
//! read the same [`EntityState`]s, so the two agree on what a paste *is*
//! even though they cannot share how it is done.
//!
//! Undo despawns what the paste created. Redo builds it again — from the
//! same captured values, so a paste undone and redone twenty times is the
//! same entity twenty times.

use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;

use crate::actions::entity_state::{self, EntityState};
use crate::undo::EditorCommand;

pub(crate) struct PasteCommand {
    /// What to build, captured at copy time.
    states: Vec<EntityState>,
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
        // 🔴 Resolved once for the whole paste, not once per entity.
        // `SpawnTarget::NewScene` makes a scene every time it is asked,
        // so resolving it inside the loop would give a clipboard of five
        // entities five scenes holding one each.
        let scene = super::place::resolve_scene(resources, self.into);
        for state in &self.states {
            let mut commands = resources.remove::<Commands>().expect("Commands not found");
            let entity = commands.spawn(resources).id();
            commands.apply(resources);
            resources.insert(commands);

            entity_state::restore_local(resources, entity, state);
            // The name is part of the copy, so the paste is not a second
            // entity called the same thing.
            if let Some(name) = entity_state::copy_name(state) {
                rename(resources, entity, &name);
            }
            // Without this the copy carries no `SceneMember`, lands
            // under "Unsaved", and is adopted by whichever scene happens
            // to be active at the next save — which is why pasting into
            // a scene used to look like it created a new one.
            if let Some(scene) = scene {
                super::place::adopt(resources, entity, scene);
            }
            self.pasted.push(entity);
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

/// Writes the `Name` component the copy was given.
fn rename(resources: &mut Resources, entity: Entity, name: &str) {
    let type_id = std::any::TypeId::of::<kooch_ecs::name::Name>();
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Err(e) = registry.reflect_set_field(
            &type_id,
            entity,
            "value",
            kooch_ecs::reflect::ReflectValue::String(name.to_owned()),
        )
    {
        tracing::debug!("the pasted entity kept its source's name: {e}");
    }
}
