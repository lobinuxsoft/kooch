//! Type-erased resource storage for the game engine.
//!
//! Resources are globally accessible singletons stored by their [`TypeId`].
//! Unlike ECS components, resources exist outside of entities and are used
//! for global state like configuration, time, and system-wide services.

use std::any::{Any, TypeId};
use std::collections::HashMap;

/// Type-erased storage for game resources.
///
/// Stores arbitrary `Send + Sync` types indexed by their [`TypeId`].
/// Provides type-safe access through generic methods.
///
/// # Example
/// ```
/// use ome_core::resource::Resources;
///
/// let mut resources = Resources::new();
/// resources.insert(42_i32);
/// resources.insert("hello".to_string());
///
/// assert_eq!(resources.get::<i32>(), Some(&42));
/// assert_eq!(resources.get::<String>(), Some(&"hello".to_string()));
/// ```
#[derive(Default)]
pub struct Resources {
    storage: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Resources {
    /// Creates an empty resource storage.
    #[inline]
    pub fn new() -> Self {
        Self {
            storage: HashMap::new(),
        }
    }

    /// Inserts a resource, replacing any existing resource of the same type.
    ///
    /// Returns the previous value if one existed.
    pub fn insert<T: Send + Sync + 'static>(&mut self, resource: T) -> Option<T> {
        self.storage
            .insert(TypeId::of::<T>(), Box::new(resource))
            .and_then(|boxed| boxed.downcast().ok().map(|b| *b))
    }

    /// Returns a reference to the resource of type `T`, if it exists.
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.storage
            .get(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_ref())
    }

    /// Returns a mutable reference to the resource of type `T`, if it exists.
    pub fn get_mut<T: Send + Sync + 'static>(&mut self) -> Option<&mut T> {
        self.storage
            .get_mut(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_mut())
    }

    /// Removes and returns the resource of type `T`, if it exists.
    pub fn remove<T: Send + Sync + 'static>(&mut self) -> Option<T> {
        self.storage
            .remove(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast().ok().map(|b| *b))
    }

    /// Returns `true` if a resource of type `T` exists.
    pub fn contains<T: Send + Sync + 'static>(&self) -> bool {
        self.storage.contains_key(&TypeId::of::<T>())
    }

    /// Returns the number of resources stored.
    #[inline]
    pub fn len(&self) -> usize {
        self.storage.len()
    }

    /// Returns `true` if no resources are stored.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.storage.is_empty()
    }

    /// Clears all resources.
    pub fn clear(&mut self) {
        self.storage.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_get() {
        let mut resources = Resources::new();
        resources.insert(42_i32);
        resources.insert("hello".to_string());

        assert_eq!(resources.get::<i32>(), Some(&42));
        assert_eq!(resources.get::<String>(), Some(&"hello".to_string()));
        assert_eq!(resources.get::<f32>(), None);
    }

    #[test]
    fn insert_replaces_existing() {
        let mut resources = Resources::new();
        resources.insert(1_i32);
        let old = resources.insert(2_i32);

        assert_eq!(old, Some(1));
        assert_eq!(resources.get::<i32>(), Some(&2));
    }

    #[test]
    fn get_mut() {
        let mut resources = Resources::new();
        resources.insert(vec![1, 2, 3]);

        if let Some(v) = resources.get_mut::<Vec<i32>>() {
            v.push(4);
        }

        assert_eq!(resources.get::<Vec<i32>>(), Some(&vec![1, 2, 3, 4]));
    }

    #[test]
    fn remove() {
        let mut resources = Resources::new();
        resources.insert(42_i32);

        let removed = resources.remove::<i32>();
        assert_eq!(removed, Some(42));
        assert!(!resources.contains::<i32>());
    }

    #[test]
    fn contains() {
        let mut resources = Resources::new();
        assert!(!resources.contains::<i32>());

        resources.insert(42_i32);
        assert!(resources.contains::<i32>());
    }

    #[test]
    fn len_and_is_empty() {
        let mut resources = Resources::new();
        assert!(resources.is_empty());
        assert_eq!(resources.len(), 0);

        resources.insert(42_i32);
        assert!(!resources.is_empty());
        assert_eq!(resources.len(), 1);

        resources.insert("hello".to_string());
        assert_eq!(resources.len(), 2);
    }
}
