//! Builder for spawning a new entity with components.

use crate::archetype_registry::ArchetypeRegistry;
use crate::component::registry::ComponentRegistry;
use crate::component::traits::Component;
use crate::entity::Entity;
use crate::reflect::Reflect;

use super::command::{Command, InsertFn};

/// Builder for spawning a new entity with components.
///
/// Component insertions are committed to the command queue when this
/// builder is dropped (or when [`id`](EntityBuilder::id) is called).
pub struct EntityBuilder<'a> {
    pub(super) entity: Entity,
    pub(super) inserts: Vec<InsertFn>,
    pub(super) queue: &'a mut Vec<Command>,
}

impl EntityBuilder<'_> {
    /// Adds a CPU component to the entity being spawned.
    pub fn insert<T: Component>(mut self, value: T) -> Self {
        self.inserts.push(Box::new(
            move |entity: Entity,
                  components: &mut ComponentRegistry,
                  archetypes: &mut ArchetypeRegistry| {
                components.register_cpu::<T>();
                components.get_cpu_mut::<T>().unwrap().insert(entity, value);
                let current = archetypes.entity_archetype(entity).unwrap();
                let new_arch = archetypes.archetype_after_add::<T>(current);
                archetypes.register_entity(entity, new_arch);
            },
        ));
        self
    }

    /// Adds a CPU component with reflection support.
    pub fn insert_reflected<T: Component + Reflect>(mut self, value: T) -> Self {
        self.inserts.push(Box::new(
            move |entity: Entity,
                  components: &mut ComponentRegistry,
                  archetypes: &mut ArchetypeRegistry| {
                components.register_cpu_reflected::<T>();
                components.get_cpu_mut::<T>().unwrap().insert(entity, value);
                let current = archetypes.entity_archetype(entity).unwrap();
                let new_arch = archetypes.archetype_after_add::<T>(current);
                archetypes.register_entity(entity, new_arch);
            },
        ));
        self
    }

    /// Finalises the builder and returns the pre-allocated entity ID.
    pub fn id(self) -> Entity {
        self.entity
    }
}

impl Drop for EntityBuilder<'_> {
    fn drop(&mut self) {
        let inserts = std::mem::take(&mut self.inserts);
        if !inserts.is_empty() {
            self.queue.push(Command::InsertComponents {
                entity: self.entity,
                inserts,
            });
        }
    }
}
