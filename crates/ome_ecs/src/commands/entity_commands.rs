//! Builder for modifying an existing entity.

use std::any::TypeId;

use crate::archetype_registry::ArchetypeRegistry;
use crate::component::registry::ComponentRegistry;
use crate::component::traits::{Component, GpuComponent};
use crate::entity::Entity;
use crate::reflect::Reflect;

use super::command::{Command, InsertFn};

/// Builder for modifying an existing entity.
///
/// Operations are committed to the command queue on drop.
pub struct EntityCommands<'a> {
    pub(super) entity: Entity,
    pub(super) inserts: Vec<InsertFn>,
    pub(super) removals: Vec<TypeId>,
    pub(super) queue: &'a mut Vec<Command>,
}

impl EntityCommands<'_> {
    /// Adds a CPU component to the entity.
    pub fn insert<T: Component>(&mut self, value: T) -> &mut Self {
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

    /// Adds a GPU component to the entity.
    pub fn insert_gpu<T: GpuComponent>(&mut self, value: T) -> &mut Self {
        self.inserts.push(Box::new(
            move |entity: Entity,
                  components: &mut ComponentRegistry,
                  archetypes: &mut ArchetypeRegistry| {
                components.register_gpu::<T>(std::any::type_name::<T>());
                components.get_gpu_mut::<T>().unwrap().insert(entity, value);
                let current = archetypes.entity_archetype(entity).unwrap();
                let new_arch = archetypes.archetype_after_add::<T>(current);
                archetypes.register_entity(entity, new_arch);
            },
        ));
        self
    }

    /// Adds a CPU component with reflection support.
    pub fn insert_reflected<T: Component + Reflect>(&mut self, value: T) -> &mut Self {
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

    /// Adds a GPU component with reflection support.
    pub fn insert_gpu_reflected<T: GpuComponent + Reflect>(&mut self, value: T) -> &mut Self {
        self.inserts.push(Box::new(
            move |entity: Entity,
                  components: &mut ComponentRegistry,
                  archetypes: &mut ArchetypeRegistry| {
                components.register_gpu_reflected::<T>(std::any::type_name::<T>());
                components.get_gpu_mut::<T>().unwrap().insert(entity, value);
                let current = archetypes.entity_archetype(entity).unwrap();
                let new_arch = archetypes.archetype_after_add::<T>(current);
                archetypes.register_entity(entity, new_arch);
            },
        ));
        self
    }

    /// Removes a component type from the entity.
    pub fn remove<T: 'static>(&mut self) -> &mut Self {
        self.removals.push(TypeId::of::<T>());
        self
    }

    /// Queues the entity for despawn.
    pub fn despawn(self) {
        // Skip the Drop impl — despawn replaces all pending inserts/removals.
        let entity = self.entity;
        let queue = unsafe { &mut *(self.queue as *mut Vec<Command>) };
        std::mem::forget(self);
        queue.push(Command::Despawn(entity));
    }
}

impl Drop for EntityCommands<'_> {
    fn drop(&mut self) {
        let inserts = std::mem::take(&mut self.inserts);
        if !inserts.is_empty() {
            self.queue.push(Command::InsertComponents {
                entity: self.entity,
                inserts,
            });
        }
        for type_id in std::mem::take(&mut self.removals) {
            self.queue.push(Command::RemoveComponent {
                entity: self.entity,
                type_id,
            });
        }
    }
}
