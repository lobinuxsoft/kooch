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

    /// Returns the resource of type `T`, inserting `default()` first if
    /// it is not there yet.
    ///
    /// For resources several plugins *contribute to* rather than own —
    /// a list of handlers, a registry. Whoever asks first creates it and
    /// the rest add to the same one, with no ordering rule between them.
    /// The alternative every call site would otherwise write is a
    /// `contains` check followed by an `insert` and an `unwrap`, which
    /// is the same thing with a panic in it.
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

    /// Returns an immutable raw pointer to the resource with the given `TypeId`.
    ///
    /// Returns null if the type is not stored.
    ///
    /// # Safety
    ///
    /// The caller must ensure the pointer is cast to the correct concrete type
    /// and not used after the resource is removed or mutably accessed.
    pub fn get_ptr_by_id(&self, type_id: TypeId) -> *const () {
        self.storage
            .get(&type_id)
            .map_or(std::ptr::null(), |boxed| {
                boxed.as_ref() as *const dyn Any as *const ()
            })
    }

    /// Returns a mutable raw pointer to the resource with the given `TypeId`.
    ///
    /// Returns null if the type is not stored.
    ///
    /// # Safety
    ///
    /// The caller must ensure the pointer is cast to the correct concrete type,
    /// not aliased, and not used after the resource is removed.
    pub fn get_mut_ptr_by_id(&mut self, type_id: TypeId) -> *mut () {
        self.storage
            .get_mut(&type_id)
            .map_or(std::ptr::null_mut(), |boxed| {
                boxed.as_mut() as *mut dyn Any as *mut ()
            })
    }
}

#[cfg(test)]
mod tests;
