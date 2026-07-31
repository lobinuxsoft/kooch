//! Marker registry for entities excluded from scene persistence.
//!
//! Entities whose archetype contains at least one type registered here are
//! treated as *ephemeral*: they are skipped by [`SceneDocument::from_ecs`]
//! during save and preserved by `despawn_all` during load. This lets editor
//! crates spawn helper entities (cameras, gizmos, grids) into the live ECS
//! without polluting the user's scene files or losing them on scene reload.
//!
//! Downstream crates register their marker `TypeId`s at startup, typically
//! by inserting `TypeId::of::<MyMarker>()` into the resource.
//!
//! [`SceneDocument::from_ecs`]: crate::scene::SceneDocument::from_ecs

use std::any::TypeId;
use std::collections::HashSet;

/// Registry of marker component types whose entities should be excluded
/// from scene serialization and preserved across scene loads.
///
/// # Example
///
/// ```ignore
/// use std::any::TypeId;
/// use kooch_ecs::ephemeral::EphemeralComponents;
///
/// struct EditorOnly;
/// impl kooch_ecs::Component for EditorOnly {}
///
/// let mut ephemeral = EphemeralComponents::new();
/// ephemeral.insert(TypeId::of::<EditorOnly>());
/// // Now any entity carrying `EditorOnly` is excluded from scene save/load.
/// ```
#[derive(Debug, Default, Clone)]
pub struct EphemeralComponents {
    types: HashSet<TypeId>,
}

impl EphemeralComponents {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            types: HashSet::new(),
        }
    }

    /// Marks a component type as ephemeral. Idempotent.
    pub fn insert(&mut self, type_id: TypeId) {
        self.types.insert(type_id);
    }

    /// Returns whether a component type is registered as ephemeral.
    pub fn contains(&self, type_id: &TypeId) -> bool {
        self.types.contains(type_id)
    }

    /// Returns the underlying set of registered marker types.
    pub fn types(&self) -> &HashSet<TypeId> {
        &self.types
    }

    /// Returns whether the given component set contains any ephemeral marker.
    ///
    /// Used by scene serialization to decide whether to skip an entire
    /// archetype (all entities in the archetype share the same component set).
    pub fn intersects<'a, I>(&self, components: I) -> bool
    where
        I: IntoIterator<Item = &'a TypeId>,
    {
        components.into_iter().any(|t| self.types.contains(t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MarkerA;
    struct MarkerB;
    struct Other;

    #[test]
    fn empty_registry_does_not_match() {
        let registry = EphemeralComponents::new();
        assert!(!registry.contains(&TypeId::of::<MarkerA>()));
        assert!(!registry.intersects([&TypeId::of::<MarkerA>(), &TypeId::of::<Other>()]));
    }

    #[test]
    fn insert_and_contains() {
        let mut registry = EphemeralComponents::new();
        registry.insert(TypeId::of::<MarkerA>());
        assert!(registry.contains(&TypeId::of::<MarkerA>()));
        assert!(!registry.contains(&TypeId::of::<MarkerB>()));
    }

    #[test]
    fn intersects_detects_any_match() {
        let mut registry = EphemeralComponents::new();
        registry.insert(TypeId::of::<MarkerA>());

        let with_marker = [TypeId::of::<MarkerA>(), TypeId::of::<Other>()];
        let without_marker = [TypeId::of::<MarkerB>(), TypeId::of::<Other>()];

        assert!(registry.intersects(with_marker.iter()));
        assert!(!registry.intersects(without_marker.iter()));
    }

    #[test]
    fn insert_is_idempotent() {
        let mut registry = EphemeralComponents::new();
        registry.insert(TypeId::of::<MarkerA>());
        registry.insert(TypeId::of::<MarkerA>());
        assert_eq!(registry.types().len(), 1);
    }
}
