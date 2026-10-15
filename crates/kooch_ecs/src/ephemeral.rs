//! Marker registry for entities excluded from scene persistence.

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
    pub fn intersects<'a, I>(&self, components: I) -> bool
    where
        I: IntoIterator<Item = &'a TypeId>,
    {
        components.into_iter().any(|t| self.types.contains(t))
    }
}

/// Every entity whose archetype carries an ephemeral marker, sorted by index.
///
/// 🔴 Walks the ARCHETYPES, not the entities: whether something is ephemeral is decided per
/// archetype, so this is one test per block instead of one per entity. Sorted so a caller filtering
/// a long column can binary-search it rather than ask the registry entity by entity — the remote's
/// moved-transform path walks every `Transform` there is, and that is the one it must not slow.
///
/// Empty when nothing is registered, which is the common case and costs one lookup.
pub fn entities(resources: &kooch_core::resource::Resources) -> Vec<crate::entity::Entity> {
    let Some(ephemeral) = resources.get::<EphemeralComponents>() else {
        return Vec::new();
    };
    if ephemeral.types().is_empty() {
        return Vec::new();
    }
    let Some(archetypes) = resources.get::<crate::archetype_registry::ArchetypeRegistry>() else {
        return Vec::new();
    };
    let mut out: Vec<crate::entity::Entity> = archetypes
        .iter_matching(&[])
        .filter(|archetype| ephemeral.intersects(archetype.components()))
        .flat_map(|archetype| archetype.entities().iter().copied())
        .collect();
    out.sort_unstable_by_key(|entity| (entity.index(), entity.generation()));
    out
}

/// Whether `entity` is in a sorted list from [`entities`].
pub fn listed(sorted: &[crate::entity::Entity], entity: crate::entity::Entity) -> bool {
    sorted
        .binary_search_by_key(&(entity.index(), entity.generation()), |held| {
            (held.index(), held.generation())
        })
        .is_ok()
}

#[cfg(test)]
mod tests;
