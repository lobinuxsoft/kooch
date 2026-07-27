//! Maps canonical resource names to [`TypeId`] for FFI access.
//!
//! Dynamic plugins identify resources by string name (e.g. `"ome_core::Time"`).
//! The engine registers known resources at startup so the bridge can resolve
//! names to `TypeId` for raw pointer access.

use std::any::TypeId;
use std::collections::HashMap;

/// Maps resource name strings to their `TypeId`.
///
/// Inserted into [`Resources`](crate::resource::Resources) by the dynamic
/// plugin loader. Bridge functions look up types here when a plugin requests
/// a resource by name.
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
    /// registry.register::<Time>("ome_core::Time");
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
mod tests {
    use super::*;

    #[test]
    fn register_and_lookup() {
        let mut reg = ResourceRegistry::new();
        reg.register::<i32>("core::i32");
        reg.register::<String>("std::String");

        assert_eq!(reg.get_type_id("core::i32"), Some(TypeId::of::<i32>()));
        assert_eq!(reg.get_type_id("std::String"), Some(TypeId::of::<String>()));
        assert_eq!(reg.get_type_id("unknown"), None);
    }

    #[test]
    fn overwrite_existing() {
        let mut reg = ResourceRegistry::new();
        reg.register::<i32>("my_type");
        reg.register::<f32>("my_type");

        assert_eq!(reg.get_type_id("my_type"), Some(TypeId::of::<f32>()));
    }

    #[test]
    fn len_and_empty() {
        let mut reg = ResourceRegistry::new();
        assert!(reg.is_empty());

        reg.register::<i32>("i32");
        assert_eq!(reg.len(), 1);
        assert!(!reg.is_empty());
    }
}
