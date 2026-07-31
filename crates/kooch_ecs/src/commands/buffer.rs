//! Public [`Commands`] buffer that collects deferred ECS mutations.

use kooch_core::resource::Resources;

use crate::allocator::EntityAllocator;
use crate::archetype::ArchetypeId;
use crate::archetype_registry::ArchetypeRegistry;
use crate::component::registry::ComponentRegistry;

use super::command::Command;
use super::entity_builder::EntityBuilder;
use super::entity_commands::EntityCommands;

/// Deferred command buffer for safe ECS mutations.
///
/// Collected commands are applied atomically via [`apply`](Commands::apply)
/// or the built-in [`commands_apply_system`](super::commands_apply_system).
pub struct Commands {
    pub(super) queue: Vec<Command>,
}

impl Commands {
    /// Creates an empty command buffer.
    pub fn new() -> Self {
        Self { queue: Vec::new() }
    }

    /// Spawns a new entity and returns a builder for adding components.
    ///
    /// The entity ID is allocated **immediately** from [`EntityAllocator`]
    /// and registered in the empty archetype. Component insertions are
    /// deferred until [`apply`](Commands::apply).
    ///
    /// # Panics
    ///
    /// Panics if `EntityAllocator` or `ArchetypeRegistry` is missing from resources.
    pub fn spawn(&mut self, resources: &mut Resources) -> EntityBuilder<'_> {
        let entity = resources
            .get_mut::<EntityAllocator>()
            .expect("EntityAllocator not found in Resources")
            .spawn();
        resources
            .get_mut::<ArchetypeRegistry>()
            .expect("ArchetypeRegistry not found in Resources")
            .register_entity(entity, ArchetypeId::EMPTY);

        EntityBuilder {
            entity,
            inserts: Vec::new(),
            queue: &mut self.queue,
        }
    }

    /// Spawns multiple entities in a batch, calling `init` for each to
    /// provide its components via an [`EntityBuilder`].
    ///
    /// Returns the allocated entity IDs.
    pub fn spawn_batch(
        &mut self,
        resources: &mut Resources,
        count: usize,
        mut init: impl FnMut(usize, EntityBuilder<'_>),
    ) -> Vec<crate::entity::Entity> {
        let mut entities = Vec::with_capacity(count);
        for i in 0..count {
            let builder = self.spawn(resources);
            let entity = builder.entity;
            entities.push(entity);
            init(i, builder);
        }
        entities
    }

    /// Returns a builder for modifying an existing entity.
    ///
    /// Component insertions and removals are deferred until
    /// [`apply`](Commands::apply).
    pub fn entity(&mut self, entity: crate::entity::Entity) -> EntityCommands<'_> {
        EntityCommands {
            entity,
            inserts: Vec::new(),
            removals: Vec::new(),
            queue: &mut self.queue,
        }
    }

    /// Queues an entity for despawn.
    ///
    /// The actual deallocation happens during [`apply`](Commands::apply).
    /// Component cleanup is handled by the existing
    /// `component_despawn_cleanup_system`.
    pub fn despawn(&mut self, entity: crate::entity::Entity) {
        self.queue.push(Command::Despawn(entity));
    }

    /// Applies all queued commands to the ECS.
    ///
    /// Removes `ComponentRegistry`, `ArchetypeRegistry`, and `EntityAllocator`
    /// from resources temporarily, applies commands, then restores them.
    ///
    /// # Panics
    ///
    /// Panics if required resources are missing.
    pub fn apply(&mut self, resources: &mut Resources) {
        if self.queue.is_empty() {
            return;
        }

        let mut components = resources
            .remove::<ComponentRegistry>()
            .expect("ComponentRegistry not found");
        let mut archetypes = resources
            .remove::<ArchetypeRegistry>()
            .expect("ArchetypeRegistry not found");

        for command in self.queue.drain(..) {
            match command {
                Command::InsertComponents { entity, inserts } => {
                    for insert in inserts {
                        insert(entity, &mut components, &mut archetypes);
                    }
                }
                Command::Despawn(entity) => {
                    // Remove from archetype tracking.
                    archetypes.unregister_entity(entity);
                    // Remove all components.
                    components.remove_entity(entity);
                    // Mark slot as dead in allocator.
                    if let Some(alloc) = resources.get_mut::<EntityAllocator>() {
                        alloc.despawn(entity);
                    }
                }
                Command::RemoveComponent { entity, type_id } => {
                    components.remove_component(entity, &type_id);
                    // Transition archetype.
                    if let Some(current) = archetypes.entity_archetype(entity) {
                        let new_arch = archetypes.archetype_after_remove_dynamic(current, type_id);
                        archetypes.register_entity(entity, new_arch);
                    }
                }
            }
        }

        resources.insert(components);
        resources.insert(archetypes);
    }

    /// Returns `true` if there are no pending commands.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Returns the number of pending commands.
    pub fn len(&self) -> usize {
        self.queue.len()
    }
}

impl Default for Commands {
    fn default() -> Self {
        Self::new()
    }
}
