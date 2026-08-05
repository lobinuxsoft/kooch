//! Archetype definition for the ECS.
//!
//! An [`Archetype`] groups entities that share the exact same set of
//! component types.  [`ArchetypeId`] uniquely identifies each combination
//! using a deterministic hash of the sorted `TypeId` set.

use std::any::TypeId;
use std::collections::BTreeSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::entity::Entity;

/// Unique identifier for an archetype, derived from its component set.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ArchetypeId(u64);

impl ArchetypeId {
    /// The empty archetype (no components).
    pub const EMPTY: Self = Self(0);

    /// Computes the archetype ID from a sorted set of component `TypeId`s.
    ///
    /// An empty set always returns [`ArchetypeId::EMPTY`].
    /// `BTreeSet` guarantees deterministic iteration order, so the hash is
    /// stable for the same component combination within a single run.
    pub fn from_components(components: &BTreeSet<TypeId>) -> Self {
        if components.is_empty() {
            return Self::EMPTY;
        }

        let mut hasher = DefaultHasher::new();
        for component in components {
            component.hash(&mut hasher);
        }
        Self(hasher.finish())
    }
}

/// A group of entities that share the same set of component types.
pub struct Archetype {
    id: ArchetypeId,
    components: BTreeSet<TypeId>,
    entities: Vec<Entity>,
}

impl Archetype {
    /// Creates a new archetype for the given component set.
    pub fn new(components: BTreeSet<TypeId>) -> Self {
        let id = ArchetypeId::from_components(&components);
        Self {
            id,
            components,
            entities: Vec::new(),
        }
    }

    /// Returns this archetype's unique identifier.
    #[inline]
    pub fn id(&self) -> ArchetypeId {
        self.id
    }

    /// Returns the set of component types in this archetype.
    #[inline]
    pub fn components(&self) -> &BTreeSet<TypeId> {
        &self.components
    }

    /// Returns the entities currently in this archetype.
    #[inline]
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    /// Returns the number of entities in this archetype.
    #[inline]
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Returns `true` if this archetype contains no entities.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Returns `true` if this archetype includes the component type `T`.
    pub fn has_component<T: 'static>(&self) -> bool {
        self.components.contains(&TypeId::of::<T>())
    }

    /// Adds an entity to this archetype.
    pub fn add_entity(&mut self, entity: Entity) {
        self.entities.push(entity);
    }

    /// Reorders this archetype's entities to follow `rank`.
    ///
    /// Iteration order is observable — systems run over it, and a client
    /// reading the world sees it — so restoring a snapshot has to put it
    /// back, not just the entities themselves. Entities absent from
    /// `rank` sort last, keeping their relative order.
    pub fn reorder_entities(&mut self, rank: &std::collections::HashMap<Entity, usize>) {
        self.entities
            .sort_by_key(|e| rank.get(e).copied().unwrap_or(usize::MAX));
    }

    /// Removes an entity from this archetype using swap-remove.
    ///
    /// Returns `true` if the entity was found and removed.
    pub fn remove_entity(&mut self, entity: Entity) -> bool {
        if let Some(pos) = self.entities.iter().position(|e| *e == entity) {
            self.entities.swap_remove(pos);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Position;
    struct Velocity;
    struct Health;

    #[test]
    fn empty_archetype_id() {
        let id = ArchetypeId::from_components(&BTreeSet::new());
        assert_eq!(id, ArchetypeId::EMPTY);
    }

    #[test]
    fn deterministic_id() {
        let mut set_a = BTreeSet::new();
        set_a.insert(TypeId::of::<Position>());
        set_a.insert(TypeId::of::<Velocity>());

        let mut set_b = BTreeSet::new();
        set_b.insert(TypeId::of::<Velocity>());
        set_b.insert(TypeId::of::<Position>());

        assert_eq!(
            ArchetypeId::from_components(&set_a),
            ArchetypeId::from_components(&set_b),
        );
    }

    #[test]
    fn different_components_different_id() {
        let mut set_a = BTreeSet::new();
        set_a.insert(TypeId::of::<Position>());

        let mut set_b = BTreeSet::new();
        set_b.insert(TypeId::of::<Velocity>());

        assert_ne!(
            ArchetypeId::from_components(&set_a),
            ArchetypeId::from_components(&set_b),
        );
    }

    #[test]
    fn non_empty_differs_from_empty() {
        let mut set = BTreeSet::new();
        set.insert(TypeId::of::<Position>());

        assert_ne!(ArchetypeId::from_components(&set), ArchetypeId::EMPTY);
    }

    #[test]
    fn archetype_new() {
        let mut components = BTreeSet::new();
        components.insert(TypeId::of::<Position>());
        components.insert(TypeId::of::<Velocity>());

        let arch = Archetype::new(components.clone());
        assert_eq!(arch.id(), ArchetypeId::from_components(&components));
        assert_eq!(arch.components(), &components);
        assert!(arch.is_empty());
        assert_eq!(arch.len(), 0);
    }

    #[test]
    fn has_component() {
        let mut components = BTreeSet::new();
        components.insert(TypeId::of::<Position>());

        let arch = Archetype::new(components);
        assert!(arch.has_component::<Position>());
        assert!(!arch.has_component::<Velocity>());
    }

    #[test]
    fn add_and_remove_entity() {
        let mut arch = Archetype::new(BTreeSet::new());
        let e1 = Entity::new(0, 0);
        let e2 = Entity::new(1, 0);

        arch.add_entity(e1);
        arch.add_entity(e2);
        assert_eq!(arch.len(), 2);
        assert_eq!(arch.entities(), &[e1, e2]);

        assert!(arch.remove_entity(e1));
        assert_eq!(arch.len(), 1);
        assert_eq!(arch.entities(), &[e2]);

        assert!(!arch.remove_entity(e1));
    }

    #[test]
    fn swap_remove_preserves_density() {
        let mut arch = Archetype::new(BTreeSet::new());
        let e1 = Entity::new(0, 0);
        let e2 = Entity::new(1, 0);
        let e3 = Entity::new(2, 0);

        arch.add_entity(e1);
        arch.add_entity(e2);
        arch.add_entity(e3);

        // Removing e1 should swap e3 into position 0.
        arch.remove_entity(e1);
        assert_eq!(arch.len(), 2);
        assert!(arch.entities().contains(&e2));
        assert!(arch.entities().contains(&e3));
    }
}
