use std::any::TypeId;
use std::collections::{BTreeSet, HashMap};

use crate::archetype::{Archetype, ArchetypeId};
use crate::entity::Entity;

/// Central registry of all archetypes and entity-archetype mappings.
///
/// Maintains a transition cache so that repeated add/remove component
/// operations resolve in O(1) after the first occurrence.
pub struct ArchetypeRegistry {
    /// All known archetypes.
    archetypes: HashMap<ArchetypeId, Archetype>,
    /// Entity → current archetype.
    entity_archetype: HashMap<Entity, ArchetypeId>,
    /// Cache: `(from_archetype, +component_type)` → `to_archetype`.
    add_transitions: HashMap<(ArchetypeId, TypeId), ArchetypeId>,
    /// Cache: `(from_archetype, -component_type)` → `to_archetype`.
    remove_transitions: HashMap<(ArchetypeId, TypeId), ArchetypeId>,
}

impl ArchetypeRegistry {
    /// Creates a new registry with the empty archetype pre-registered.
    pub fn new() -> Self {
        let mut archetypes = HashMap::new();
        archetypes.insert(ArchetypeId::EMPTY, Archetype::new(BTreeSet::new()));

        Self {
            archetypes,
            entity_archetype: HashMap::new(),
            add_transitions: HashMap::new(),
            remove_transitions: HashMap::new(),
        }
    }

    /// Returns the archetype for a component set, creating it if needed.
    pub fn get_or_create(&mut self, components: BTreeSet<TypeId>) -> ArchetypeId {
        let id = ArchetypeId::from_components(&components);

        self.archetypes
            .entry(id)
            .or_insert_with(|| Archetype::new(components));

        id
    }

    /// Returns an immutable reference to an archetype.
    pub fn get(&self, id: ArchetypeId) -> Option<&Archetype> {
        self.archetypes.get(&id)
    }

    /// Returns the archetype an entity currently belongs to.
    pub fn entity_archetype(&self, entity: Entity) -> Option<ArchetypeId> {
        self.entity_archetype.get(&entity).copied()
    }

    /// Moves an entity into the given archetype.
    ///
    /// If the entity was already in a different archetype, it is removed
    /// from the old one first.
    pub fn register_entity(&mut self, entity: Entity, archetype_id: ArchetypeId) {
        if let Some(old_id) = self.entity_archetype.get(&entity).copied() {
            if old_id == archetype_id {
                return;
            }
            if let Some(archetype) = self.archetypes.get_mut(&old_id) {
                archetype.remove_entity(entity);
            }
        }

        if let Some(archetype) = self.archetypes.get_mut(&archetype_id) {
            archetype.add_entity(entity);
        }

        self.entity_archetype.insert(entity, archetype_id);
    }

    /// Removes an entity from its current archetype entirely.
    ///
    /// Returns the archetype it was removed from, or `None` if untracked.
    pub fn unregister_entity(&mut self, entity: Entity) -> Option<ArchetypeId> {
        if let Some(old_id) = self.entity_archetype.remove(&entity) {
            if let Some(archetype) = self.archetypes.get_mut(&old_id) {
                archetype.remove_entity(entity);
            }
            Some(old_id)
        } else {
            None
        }
    }

    /// Computes the archetype that results from adding component `T` to
    /// `current`, using the transition cache.
    pub fn archetype_after_add<T: 'static>(&mut self, current: ArchetypeId) -> ArchetypeId {
        self.archetype_after_add_dynamic(current, TypeId::of::<T>())
    }

    /// Computes the archetype that results from removing component `T` from
    /// `current`, using the transition cache.
    pub fn archetype_after_remove<T: 'static>(&mut self, current: ArchetypeId) -> ArchetypeId {
        self.archetype_after_remove_dynamic(current, TypeId::of::<T>())
    }

    /// Like [`archetype_after_add`](Self::archetype_after_add) but takes a
    /// `TypeId` directly.
    pub fn archetype_after_add_dynamic(
        &mut self,
        current: ArchetypeId,
        type_id: TypeId,
    ) -> ArchetypeId {
        let key = (current, type_id);

        if let Some(&cached) = self.add_transitions.get(&key) {
            return cached;
        }

        let mut new_components = self.archetypes[&current].components().clone();
        new_components.insert(type_id);

        let new_id = self.get_or_create(new_components);
        self.add_transitions.insert(key, new_id);
        new_id
    }

    /// Like [`archetype_after_remove`](Self::archetype_after_remove) but takes
    /// a `TypeId` directly.
    pub fn archetype_after_remove_dynamic(
        &mut self,
        current: ArchetypeId,
        type_id: TypeId,
    ) -> ArchetypeId {
        let key = (current, type_id);

        if let Some(&cached) = self.remove_transitions.get(&key) {
            return cached;
        }

        let mut new_components = self.archetypes[&current].components().clone();
        new_components.remove(&type_id);

        let new_id = self.get_or_create(new_components);
        self.remove_transitions.insert(key, new_id);
        new_id
    }

    /// Iterates over archetypes that contain **all** of the `required`
    /// component types.
    pub fn iter_matching(&self, required: &[TypeId]) -> impl Iterator<Item = &Archetype> {
        self.archetypes
            .values()
            .filter(move |arch| required.iter().all(|r| arch.components().contains(r)))
    }

    /// Reorders every archetype's entities to follow `order`.
    ///
    /// Used when restoring a snapshot: rebuilding an entity walks it
    /// through a chain of archetypes, and where it lands in each one
    /// depends on the order components happened to be added — which is
    /// not the order the world had. This puts the observable iteration
    /// order back.
    pub fn reorder_entities(&mut self, order: &[Entity]) {
        let rank: std::collections::HashMap<Entity, usize> =
            order.iter().enumerate().map(|(i, e)| (*e, i)).collect();
        for archetype in self.archetypes.values_mut() {
            archetype.reorder_entities(&rank);
        }
    }

    /// Returns the total number of registered archetypes.
    pub fn archetype_count(&self) -> usize {
        self.archetypes.len()
    }

    /// Removes empty archetypes (except `EMPTY`) and invalidates
    /// transition cache entries that reference them.
    ///
    /// Call periodically or after batch entity operations to avoid
    /// unbounded archetype accumulation.
    pub fn gc_empty_archetypes(&mut self) -> usize {
        let to_remove: Vec<ArchetypeId> = self
            .archetypes
            .iter()
            .filter(|(id, arch)| **id != ArchetypeId::EMPTY && arch.len() == 0)
            .map(|(&id, _)| id)
            .collect();

        let count = to_remove.len();
        for id in &to_remove {
            self.archetypes.remove(id);
        }

        // Purge transition cache entries that reference removed archetypes.
        self.add_transitions
            .retain(|&(src, _), dst| !to_remove.contains(&src) && !to_remove.contains(dst));
        self.remove_transitions
            .retain(|&(src, _), dst| !to_remove.contains(&src) && !to_remove.contains(dst));

        count
    }
}

impl Default for ArchetypeRegistry {
    fn default() -> Self {
        Self::new()
    }
}
