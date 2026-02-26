//! CPU-only component storage backed by a `HashMap`.
//!
//! [`ComponentStorage<T>`] is for components that never touch the GPU
//! (e.g. inventory, AI state, metadata).

use std::any::Any;
use std::collections::HashMap;

use wgpu::{Device, Queue};

use crate::entity::Entity;

use super::traits::{AnyStorage, Component};

/// CPU-only component storage using a `HashMap<Entity, T>`.
pub struct ComponentStorage<T: Component> {
    data: HashMap<Entity, T>,
}

impl<T: Component> ComponentStorage<T> {
    /// Creates empty storage.
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
        }
    }

    /// Inserts a component value for `entity`, returning the previous value.
    pub fn insert(&mut self, entity: Entity, value: T) -> Option<T> {
        self.data.insert(entity, value)
    }

    /// Removes the component for `entity`, returning it if present.
    pub fn remove(&mut self, entity: Entity) -> Option<T> {
        self.data.remove(&entity)
    }

    /// Returns an immutable reference to the component, if present.
    pub fn get(&self, entity: Entity) -> Option<&T> {
        self.data.get(&entity)
    }

    /// Returns a mutable reference to the component, if present.
    pub fn get_mut(&mut self, entity: Entity) -> Option<&mut T> {
        self.data.get_mut(&entity)
    }

    /// Returns `true` if this storage has a component for `entity`.
    pub fn contains(&self, entity: Entity) -> bool {
        self.data.contains_key(&entity)
    }

    /// Number of entities with this component.
    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns `true` if no entities have this component.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Iterates over all `(entity, component)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&Entity, &T)> {
        self.data.iter()
    }

    /// Iterates mutably over all `(entity, component)` pairs.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&Entity, &mut T)> {
        self.data.iter_mut()
    }
}

impl<T: Component> Default for ComponentStorage<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Component> AnyStorage for ComponentStorage<T> {
    fn remove_entity(&mut self, entity: Entity) {
        self.data.remove(&entity);
    }

    fn sync_gpu(&mut self, _device: &Device, _queue: &Queue, _capacity: u32) {
        // No-op for CPU-only components.
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Health(u32);
    impl Component for Health {}

    fn entity(index: u32, generation: u32) -> Entity {
        Entity::new(index, generation)
    }

    #[test]
    fn insert_and_get() {
        let mut storage = ComponentStorage::<Health>::new();
        let e = entity(0, 0);

        storage.insert(e, Health(100));
        assert_eq!(storage.get(e).unwrap().0, 100);
        assert!(storage.contains(e));
        assert_eq!(storage.len(), 1);
    }

    #[test]
    fn insert_returns_previous() {
        let mut storage = ComponentStorage::<Health>::new();
        let e = entity(0, 0);

        assert!(storage.insert(e, Health(100)).is_none());
        let old = storage.insert(e, Health(200));
        assert_eq!(old.unwrap().0, 100);
        assert_eq!(storage.get(e).unwrap().0, 200);
    }

    #[test]
    fn remove_returns_value() {
        let mut storage = ComponentStorage::<Health>::new();
        let e = entity(0, 0);

        storage.insert(e, Health(50));
        let removed = storage.remove(e);
        assert_eq!(removed.unwrap().0, 50);
        assert!(!storage.contains(e));
        assert!(storage.is_empty());
    }

    #[test]
    fn remove_nonexistent_returns_none() {
        let mut storage = ComponentStorage::<Health>::new();
        assert!(storage.remove(entity(99, 0)).is_none());
    }

    #[test]
    fn get_mut_modifies() {
        let mut storage = ComponentStorage::<Health>::new();
        let e = entity(0, 0);

        storage.insert(e, Health(10));
        storage.get_mut(e).unwrap().0 = 42;
        assert_eq!(storage.get(e).unwrap().0, 42);
    }

    #[test]
    fn iter_all_entries() {
        let mut storage = ComponentStorage::<Health>::new();
        storage.insert(entity(0, 0), Health(1));
        storage.insert(entity(1, 0), Health(2));
        storage.insert(entity(2, 0), Health(3));

        let sum: u32 = storage.iter().map(|(_, h)| h.0).sum();
        assert_eq!(sum, 6);
    }

    #[test]
    fn iter_mut_modifies_all() {
        let mut storage = ComponentStorage::<Health>::new();
        storage.insert(entity(0, 0), Health(1));
        storage.insert(entity(1, 0), Health(2));

        for (_, h) in storage.iter_mut() {
            h.0 *= 10;
        }

        let sum: u32 = storage.iter().map(|(_, h)| h.0).sum();
        assert_eq!(sum, 30);
    }

    #[test]
    fn entity_keyed_by_index_and_generation() {
        let mut storage = ComponentStorage::<Health>::new();
        let e_gen0 = entity(0, 0);
        let e_gen1 = entity(0, 1);

        storage.insert(e_gen0, Health(10));
        // Different generation = different entity key.
        assert!(!storage.contains(e_gen1));
        storage.insert(e_gen1, Health(20));
        assert_eq!(storage.len(), 2);
    }

    #[test]
    fn any_storage_remove_entity() {
        let mut storage = ComponentStorage::<Health>::new();
        let e = entity(0, 0);
        storage.insert(e, Health(100));

        let any_storage: &mut dyn AnyStorage = &mut storage;
        any_storage.remove_entity(e);
        assert!(storage.is_empty());
    }
}
