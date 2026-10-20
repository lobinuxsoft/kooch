//! Archetype definition for the ECS.

use std::any::TypeId;
use std::collections::BTreeSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::entity::Entity;
use crate::storage::{TableId, TableRow};

/// Unique identifier for an archetype, derived from its component set.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ArchetypeId(u64);

impl ArchetypeId {
    /// The empty archetype (no components).
    pub const EMPTY: Self = Self(0);

    /// Computes the archetype ID from a sorted set of component `TypeId`s.
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
///
/// 🔴 The archetype hands out a **row**, not a key (#891). `entities` and `rows` are parallel: the
/// walk reads both from the same index, so a query never asks a map where an entity's values are.
pub struct Archetype {
    id: ArchetypeId,
    components: BTreeSet<TypeId>,
    entities: Vec<Entity>,
    /// Which table row holds each entity's values. Parallel to `entities`.
    ///
    /// `None` for an entity whose values still live in the per-type maps — a component of its
    /// archetype has no value yet, so the row cannot be filled without leaving a column short.
    rows: Vec<Option<TableRow>>,
    /// The table serving this component set, resolved once rather than per query.
    table: Option<TableId>,
}

impl Archetype {
    /// Creates a new archetype for the given component set.
    pub fn new(components: BTreeSet<TypeId>) -> Self {
        let id = ArchetypeId::from_components(&components);
        Self {
            id,
            components,
            entities: Vec::new(),
            rows: Vec::new(),
            table: None,
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

    /// The table row holding each entity's values, parallel to [`Self::entities`].
    #[inline]
    pub fn rows(&self) -> &[Option<TableRow>] {
        &self.rows
    }

    /// The table serving this archetype's component set, once one has been built.
    #[inline]
    pub fn table(&self) -> Option<TableId> {
        self.table
    }

    /// Records which table serves this archetype.
    #[inline]
    pub fn set_table(&mut self, table: TableId) {
        self.table = Some(table);
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

    /// Adds an entity to this archetype, returning the slot it took.
    pub fn add_entity(&mut self, entity: Entity, row: Option<TableRow>) -> u32 {
        let at = self.entities.len() as u32;
        self.entities.push(entity);
        self.rows.push(row);
        at
    }

    /// The table row at `at`, if that slot holds one.
    #[inline]
    pub fn row_at(&self, at: u32) -> Option<TableRow> {
        self.rows.get(at as usize).copied().flatten()
    }

    /// Points the slot at `at` at a different table row.
    #[inline]
    pub fn set_row(&mut self, at: u32, row: Option<TableRow>) {
        if let Some(slot) = self.rows.get_mut(at as usize) {
            *slot = row;
        }
    }

    /// Reorders this archetype's entities to follow `rank`.
    ///
    /// 🔴 Rows travel with their entity. The table's own order does not change — only which order
    /// this archetype walks it in — so the pair has to move together.
    pub fn reorder_entities(&mut self, rank: &std::collections::HashMap<Entity, usize>) {
        let mut paired: Vec<(Entity, Option<TableRow>)> = self
            .entities
            .iter()
            .copied()
            .zip(self.rows.iter().copied())
            .collect();
        paired.sort_by_key(|(entity, _)| rank.get(entity).copied().unwrap_or(usize::MAX));
        for (at, (entity, row)) in paired.into_iter().enumerate() {
            self.entities[at] = entity;
            self.rows[at] = row;
        }
    }

    /// Removes the entity in slot `at`, pulling the last one into the hole.
    ///
    /// Returns the entity dragged into `at`, which the caller has to re-file: it asked for nothing
    /// and its slot changed anyway.
    pub fn remove_at(&mut self, at: u32) -> Option<Entity> {
        let at = at as usize;
        if at >= self.entities.len() {
            return None;
        }
        self.entities.swap_remove(at);
        self.rows.swap_remove(at);
        self.entities.get(at).copied()
    }
}

#[cfg(test)]
mod tests;
