//! Type-erased resource storage for the game engine.

use std::any::{Any, TypeId};
use std::collections::HashMap;

/// Type-erased storage for game resources.
///
/// Stores arbitrary `Send + Sync` types indexed by their [`TypeId`].
/// Provides type-safe access through generic methods.
///
/// # Example
/// ```
/// use kooch_core::resource::Resources;
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

    /// Returns the resource of type `T`, inserting `default()` first if it is not there yet.
    pub fn get_or_default<T: Send + Sync + Default + 'static>(&mut self) -> &mut T {
        self.storage
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(T::default()))
            .downcast_mut()
            .expect("resource stored under its own TypeId has that type")
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
mod tests;
