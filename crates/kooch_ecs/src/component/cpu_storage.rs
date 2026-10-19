//! CPU-only component storage backed by a `HashMap`.

use std::any::Any;
use std::collections::HashMap;

use crate::entity::Entity;

use super::traits::{AnyStorage, Component};

/// CPU-only component storage using a `HashMap<Entity, T>`.
pub struct ComponentStorage<T: Component> {
    data: HashMap<Entity, T>,
    /// Bumped by every write. Read by anything that caches a view of this component and has to
    /// know whether the view is still true — the editor's per-frame world snapshot (#1561).
    revision: u64,
}

impl<T: Component> ComponentStorage<T> {
    /// Creates empty storage.
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
            revision: 0,
        }
    }

    /// How many times this storage has been written to.
    #[inline]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Inserts a component value for `entity`, returning the previous value.
    pub fn insert(&mut self, entity: Entity, value: T) -> Option<T> {
        self.revision += 1;
        self.data.insert(entity, value)
    }

    /// Removes the component for `entity`, returning it if present.
    pub fn remove(&mut self, entity: Entity) -> Option<T> {
        self.revision += 1;
        self.data.remove(&entity)
    }

    /// Returns an immutable reference to the component, if present.
    pub fn get(&self, entity: Entity) -> Option<&T> {
        self.data.get(&entity)
    }

    /// Returns a mutable reference to the component, if present.
    pub fn get_mut(&mut self, entity: Entity) -> Option<&mut T> {
        // Handing out `&mut` counts as a write: whether the caller changes anything is not
        // knowable here, and a missed bump is a panel that stops updating.
        self.revision += 1;
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
        self.revision += 1;
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

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn get_ptr(&self, entity: Entity) -> Option<*const u8> {
        self.data.get(&entity).map(|v| v as *const T as *const u8)
    }

    fn get_mut_ptr(&mut self, entity: Entity) -> Option<*mut u8> {
        self.data.get_mut(&entity).map(|v| v as *mut T as *mut u8)
    }
}

#[cfg(test)]
mod tests;
