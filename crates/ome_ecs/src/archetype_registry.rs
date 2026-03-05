//! Archetype registry — tracks entity-to-archetype mappings and caches
//! archetype transitions.
//!
//! [`ArchetypeRegistry`] is the central index that knows which archetype
//! every entity belongs to and provides efficient archetype-based iteration.

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
        self.archetypes.values().filter(move |arch| {
            required.iter().all(|r| arch.components().contains(r))
        })
    }

    /// Returns the total number of registered archetypes.
    pub fn archetype_count(&self) -> usize {
        self.archetypes.len()
    }
}

impl Default for ArchetypeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Position;
    struct Velocity;
    struct Health;

    fn entity(index: u32) -> Entity {
        Entity::new(index, 0)
    }

    #[test]
    fn new_has_empty_archetype() {
        let reg = ArchetypeRegistry::new();
        assert_eq!(reg.archetype_count(), 1);
        assert!(reg.get(ArchetypeId::EMPTY).is_some());
    }

    #[test]
    fn get_or_create_idempotent() {
        let mut reg = ArchetypeRegistry::new();

        let mut components = BTreeSet::new();
        components.insert(TypeId::of::<Position>());

        let id1 = reg.get_or_create(components.clone());
        let id2 = reg.get_or_create(components);

        assert_eq!(id1, id2);
        assert_eq!(reg.archetype_count(), 2); // EMPTY + Position
    }

    #[test]
    fn register_entity_into_archetype() {
        let mut reg = ArchetypeRegistry::new();
        let e1 = entity(0);

        reg.register_entity(e1, ArchetypeId::EMPTY);

        assert_eq!(reg.entity_archetype(e1), Some(ArchetypeId::EMPTY));
        assert_eq!(reg.get(ArchetypeId::EMPTY).unwrap().len(), 1);
    }

    #[test]
    fn register_entity_migrates() {
        let mut reg = ArchetypeRegistry::new();
        let e1 = entity(0);

        reg.register_entity(e1, ArchetypeId::EMPTY);

        let pos_arch = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);
        reg.register_entity(e1, pos_arch);

        assert_eq!(reg.entity_archetype(e1), Some(pos_arch));
        assert_eq!(reg.get(ArchetypeId::EMPTY).unwrap().len(), 0);
        assert_eq!(reg.get(pos_arch).unwrap().len(), 1);
    }

    #[test]
    fn register_same_archetype_is_noop() {
        let mut reg = ArchetypeRegistry::new();
        let e1 = entity(0);

        reg.register_entity(e1, ArchetypeId::EMPTY);
        reg.register_entity(e1, ArchetypeId::EMPTY);

        assert_eq!(reg.get(ArchetypeId::EMPTY).unwrap().len(), 1);
    }

    #[test]
    fn unregister_entity() {
        let mut reg = ArchetypeRegistry::new();
        let e1 = entity(0);

        reg.register_entity(e1, ArchetypeId::EMPTY);
        let old = reg.unregister_entity(e1);

        assert_eq!(old, Some(ArchetypeId::EMPTY));
        assert!(reg.entity_archetype(e1).is_none());
        assert_eq!(reg.get(ArchetypeId::EMPTY).unwrap().len(), 0);
    }

    #[test]
    fn unregister_unknown_entity() {
        let mut reg = ArchetypeRegistry::new();
        assert_eq!(reg.unregister_entity(entity(99)), None);
    }

    #[test]
    fn archetype_after_add() {
        let mut reg = ArchetypeRegistry::new();

        let a1 = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);
        let a2 = reg.archetype_after_add::<Velocity>(a1);

        assert_ne!(a1, ArchetypeId::EMPTY);
        assert_ne!(a2, a1);

        let arch = reg.get(a2).unwrap();
        assert!(arch.has_component::<Position>());
        assert!(arch.has_component::<Velocity>());
        assert!(!arch.has_component::<Health>());
    }

    #[test]
    fn archetype_after_remove() {
        let mut reg = ArchetypeRegistry::new();

        let pos_vel = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);
        let pos_vel = reg.archetype_after_add::<Velocity>(pos_vel);
        let pos_only = reg.archetype_after_remove::<Velocity>(pos_vel);

        let arch = reg.get(pos_only).unwrap();
        assert!(arch.has_component::<Position>());
        assert!(!arch.has_component::<Velocity>());
    }

    #[test]
    fn remove_all_components_returns_to_empty() {
        let mut reg = ArchetypeRegistry::new();

        let with_pos = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);
        let back_to_empty = reg.archetype_after_remove::<Position>(with_pos);

        assert_eq!(back_to_empty, ArchetypeId::EMPTY);
    }

    #[test]
    fn transition_cache_works() {
        let mut reg = ArchetypeRegistry::new();

        let a1 = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);
        let a2 = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);

        assert_eq!(a1, a2);
        // Should not create extra archetypes.
        assert_eq!(reg.archetype_count(), 2);
    }

    #[test]
    fn iter_matching_basic() {
        let mut reg = ArchetypeRegistry::new();

        // Create archetypes: (Pos), (Pos, Vel), (Pos, Vel, Health)
        let pos = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);
        let pos_vel = reg.archetype_after_add::<Velocity>(pos);
        let pos_vel_hp = reg.archetype_after_add::<Health>(pos_vel);

        // Add entities.
        reg.register_entity(entity(0), pos);
        reg.register_entity(entity(1), pos_vel);
        reg.register_entity(entity(2), pos_vel);
        reg.register_entity(entity(3), pos_vel_hp);

        // Query for (Position, Velocity) should match pos_vel and pos_vel_hp.
        let required = [TypeId::of::<Position>(), TypeId::of::<Velocity>()];
        let matched: Vec<&Archetype> = reg.iter_matching(&required).collect();

        assert_eq!(matched.len(), 2);
        let total_entities: usize = matched.iter().map(|a| a.len()).sum();
        assert_eq!(total_entities, 3); // e1, e2, e3
    }

    #[test]
    fn iter_matching_empty_required() {
        let mut reg = ArchetypeRegistry::new();

        let pos = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);
        reg.register_entity(entity(0), pos);
        reg.register_entity(entity(1), ArchetypeId::EMPTY);

        // Empty required matches ALL archetypes.
        let matched: Vec<&Archetype> = reg.iter_matching(&[]).collect();
        assert_eq!(matched.len(), 2);
    }

    #[test]
    fn full_migration_workflow() {
        let mut reg = ArchetypeRegistry::new();
        let e = entity(0);

        // Entity starts in EMPTY archetype.
        reg.register_entity(e, ArchetypeId::EMPTY);

        // Add Position.
        let current = reg.entity_archetype(e).unwrap();
        let new_arch = reg.archetype_after_add::<Position>(current);
        reg.register_entity(e, new_arch);

        // Add Velocity.
        let current = reg.entity_archetype(e).unwrap();
        let new_arch = reg.archetype_after_add::<Velocity>(current);
        reg.register_entity(e, new_arch);

        // Verify entity has both components in its archetype.
        let arch_id = reg.entity_archetype(e).unwrap();
        let arch = reg.get(arch_id).unwrap();
        assert!(arch.has_component::<Position>());
        assert!(arch.has_component::<Velocity>());
        assert_eq!(arch.entities(), &[e]);

        // Remove Position.
        let new_arch = reg.archetype_after_remove::<Position>(arch_id);
        reg.register_entity(e, new_arch);

        let arch_id = reg.entity_archetype(e).unwrap();
        let arch = reg.get(arch_id).unwrap();
        assert!(!arch.has_component::<Position>());
        assert!(arch.has_component::<Velocity>());
    }

    #[test]
    fn dynamic_transition_methods() {
        let mut reg = ArchetypeRegistry::new();

        let type_id = TypeId::of::<Position>();
        let a1 = reg.archetype_after_add_dynamic(ArchetypeId::EMPTY, type_id);
        let a2 = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);

        assert_eq!(a1, a2);
    }

    #[test]
    fn multiple_entities_per_archetype() {
        let mut reg = ArchetypeRegistry::new();
        let arch = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);

        for i in 0..100 {
            reg.register_entity(entity(i), arch);
        }

        assert_eq!(reg.get(arch).unwrap().len(), 100);
    }
}
