//! Maps canonical resource names to [`TypeId`] for FFI access.

use std::any::TypeId;
use std::collections::HashMap;

/// Maps resource name strings to their `TypeId`.
pub struct ResourceRegistry {
    map: HashMap<String, TypeId>,
}

impl ResourceRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    /// Registers a type under a canonical name.
    ///
    /// # Example
    /// ```ignore
    /// registry.register::<Time>("kooch_core::Time");
    /// ```
    pub fn register<T: 'static>(&mut self, name: &str) {
        self.map.insert(name.to_owned(), TypeId::of::<T>());
    }

    /// Looks up the `TypeId` for a registered name.
    pub fn get_type_id(&self, name: &str) -> Option<TypeId> {
        self.map.get(name).copied()
    }

    /// Returns the number of registered types.
    #[inline]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Returns `true` if no types are registered.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl Default for ResourceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
