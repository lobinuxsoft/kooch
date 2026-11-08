//! CPU-only component storage backed by a `HashMap`.

use std::any::Any;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::entity::Entity;
use crate::storage::Column;

use super::traits::{AnyStorage, Component};

/// CPU-only component storage using a `HashMap<Entity, T>`.
/// The **inbox** for one component type: values written before their archetype caught up with
/// them, waiting to be promoted into a column (#891).
///
/// 🔴 Crate-private on purpose. A value has one home, and the two doors into this one
/// — [`ComponentViewMut::insert`](crate::component::ComponentViewMut::insert) and
/// `ComponentRegistry::insert_default_reflected` — both check for a column first. A third door
/// would give a component two values, and which one a caller saw would depend on which door it
/// came through: that is how every transform in a loaded scene ended up at the identity.
pub(crate) struct ComponentStorage<T: Component> {
    data: HashMap<Entity, T>,
    /// The change tick of each value's last write (#1870).
    changed: HashMap<Entity, AtomicU64>,
}

impl<T: Component> ComponentStorage<T> {
    /// Creates empty storage.
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
            changed: HashMap::new(),
        }
    }

    /// Inserts a component value for `entity`, returning the previous value.
    pub fn insert(&mut self, entity: Entity, value: T) -> Option<T> {
        self.stamp(entity);
        self.data.insert(entity, value)
    }

    /// Removes the component for `entity`, returning it if present.
    pub fn remove(&mut self, entity: Entity) -> Option<T> {
        self.changed.remove(&entity);
        self.data.remove(&entity)
    }

    /// Returns an immutable reference to the component, if present.
    pub fn get(&self, entity: Entity) -> Option<&T> {
        self.data.get(&entity)
    }

    /// Returns a mutable reference to the component, if present.
    pub fn get_mut(&mut self, entity: Entity) -> Option<&mut T> {
        if self.data.contains_key(&entity) {
            self.stamp(entity);
        }
        self.data.get_mut(&entity)
    }

    /// Number of entities with this component.
    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Iterates over all `(entity, component)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&Entity, &T)> {
        self.data.iter()
    }

    /// Iterates mutably over all `(entity, component)` pairs.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&Entity, &mut T)> {
        // The inbox is transient: everything in it counts as written.
        let now = kooch_core::change_tick::now();
        for tick in self.changed.values() {
            tick.store(now, Ordering::Relaxed);
        }
        self.data.iter_mut()
    }

    /// The change tick of `entity`'s value; 0 when it has none here.
    pub fn changed_at(&self, entity: Entity) -> u64 {
        self.changed
            .get(&entity)
            .map_or(0, |tick| tick.load(Ordering::Relaxed))
    }

    /// Marks `entity`'s value as written now.
    fn stamp(&mut self, entity: Entity) {
        let now = kooch_core::change_tick::now();
        match self.changed.get(&entity) {
            Some(tick) => tick.store(now, Ordering::Relaxed),
            None => {
                self.changed.insert(entity, AtomicU64::new(now));
            }
        }
    }
}

impl<T: Component> Default for ComponentStorage<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Component> AnyStorage for ComponentStorage<T> {
    fn remove_entity(&mut self, entity: Entity) {
        self.changed.remove(&entity);
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
        if let Some(tick) = self.changed.get(&entity) {
            tick.store(kooch_core::change_tick::now(), Ordering::Relaxed);
        }
        self.data.get_mut(&entity).map(|v| v as *mut T as *mut u8)
    }

    fn take_into(&mut self, entity: Entity, column: &mut Column) -> bool {
        let Some(value) = self.data.remove(&entity) else {
            return false;
        };
        self.changed.remove(&entity);
        // SAFETY: the column was built by `Column::of::<T>` for this very slot.
        unsafe { column.push(value) };
        true
    }

    fn put_from(&mut self, entity: Entity, column: &mut Column, row: usize) -> bool {
        // SAFETY: the column was built by `Column::of::<T>` for this very slot.
        let Some(value) = (unsafe { column.take::<T>(row) }) else {
            return false;
        };
        self.stamp(entity);
        self.data.insert(entity, value);
        true
    }

    fn holds(&self, entity: Entity) -> bool {
        self.data.contains_key(&entity)
    }
}

#[cfg(test)]
mod tests;
