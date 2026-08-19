use std::any::TypeId;
use std::collections::{BTreeSet, HashMap};

use crate::archetype::{Archetype, ArchetypeId};
use crate::component::ComponentRegistry;
use crate::entity::Entity;
use crate::storage::{TableId, TableRow, Tables};

/// Central registry of all archetypes and entity-archetype mappings.
///
/// Maintains a transition cache so that repeated add/remove component
/// operations resolve in O(1) after the first occurrence.
pub struct ArchetypeRegistry {
    /// All known archetypes.
    archetypes: HashMap<ArchetypeId, Archetype>,
    /// Entity → where its components live.
    entity_archetype: HashMap<Entity, ArchetypeId>,
    /// Entity → the table row holding its values, for entities that have
    /// been placed in one.
    ///
    /// ⚠️ Separate from `entity_archetype` **only while the migration of
    /// #891 is in flight**: today most entities are in an archetype and
    /// their values are still in `ComponentStorage<T>`, so they have an
    /// archetype and no row. When the last insert path moves (stage 5c-2)
    /// the two collapse into one map.
    entity_rows: HashMap<Entity, TableRow>,
    /// Cache: `(from_archetype, +component_type)` → `to_archetype`.
    add_transitions: HashMap<(ArchetypeId, TypeId), ArchetypeId>,
    /// Cache: `(from_archetype, -component_type)` → `to_archetype`.
    remove_transitions: HashMap<(ArchetypeId, TypeId), ArchetypeId>,
    /// Where the component VALUES will live (#891). Empty until something
    /// asks for a table, which is the point where the component registry
    /// is in scope and a column can actually be built.
    tables: Tables,
}

impl ArchetypeRegistry {
    /// Creates a new registry with the empty archetype pre-registered.
    pub fn new() -> Self {
        let mut archetypes = HashMap::new();
        archetypes.insert(ArchetypeId::EMPTY, Archetype::new(BTreeSet::new()));

        Self {
            archetypes,
            entity_archetype: HashMap::new(),
            entity_rows: HashMap::new(),
            add_transitions: HashMap::new(),
            remove_transitions: HashMap::new(),
            tables: Tables::new(),
        }
    }

    /// The tables holding this world's component values.
    #[inline]
    pub fn tables(&self) -> &Tables {
        &self.tables
    }

    /// The tables, mutably.
    #[inline]
    pub fn tables_mut(&mut self) -> &mut Tables {
        &mut self.tables
    }

    /// The table serving `archetype`'s component set, built on first ask.
    ///
    /// Returns `None` if the archetype is unknown.
    ///
    /// # Why this is a lookup and not a field on `Archetype`
    ///
    /// Both ids are functions of the **same component set**:
    /// `ArchetypeId::from_components` hashes the types, and
    /// [`Tables::get_or_insert`] keys on their [`StorageId`]s. Storing the
    /// table on the archetype would mean handing a `&ComponentRegistry` to
    /// every one of the 38 places that create or transition an archetype,
    /// none of which cares about storage — and a column cannot be built
    /// without the concrete type, so the registry has to be *somewhere*.
    ///
    /// It is asked for where a value is actually written, which already
    /// holds both registries.
    ///
    /// ⚠️ **And it is deliberately not cached.** [`Tables::get_or_insert`]
    /// already dedupes by component set, so a cache here would change no
    /// observable behaviour — it would only save recomputing a short
    /// `Vec<StorageId>`, an amount nobody has measured, at the price of a
    /// second structure that can drift from the first. If a capture ever
    /// says this lookup matters, cache it then, with the number.
    ///
    /// # Panics
    ///
    /// If a component of the archetype is not registered. Reaching a table
    /// for a component nothing ever registered is a bug upstream — the
    /// insert path registers before it transitions.
    ///
    /// [`StorageId`]: crate::component::StorageId
    pub fn table_of(
        &mut self,
        archetype: ArchetypeId,
        components: &ComponentRegistry,
    ) -> Option<TableId> {
        let ids: Vec<_> = self
            .archetypes
            .get(&archetype)?
            .components()
            .iter()
            .map(|type_id| {
                components
                    .storage_id(type_id)
                    .unwrap_or_else(|| panic!("component {type_id:?} is not registered"))
            })
            .collect();

        Some(self.tables.get_or_insert(components, &ids))
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

    // -- Rows (#891, stage 5c-1) ---------------------------------------------

    /// The table row holding `entity`'s values, if it has been placed.
    #[inline]
    pub fn row_of(&self, entity: Entity) -> Option<TableRow> {
        self.entity_rows.get(&entity).copied()
    }

    /// Registers `entity` in `archetype` **and** claims it a row in that
    /// archetype's table.
    ///
    /// 🔴 The caller must then push one value into every column of the
    /// table, because those values are typed and this layer is not. Until
    /// it does, the table's `rows_agree` is false.
    ///
    /// Returns the row, or `None` if the archetype is unknown.
    pub fn place(
        &mut self,
        entity: Entity,
        archetype: ArchetypeId,
        components: &ComponentRegistry,
    ) -> Option<TableRow> {
        let table = self.table_of(archetype, components)?;
        self.register_entity(entity, archetype);
        let row = self.tables.get_mut(table)?.push_entity(entity);
        self.entity_rows.insert(entity, row);
        Some(row)
    }

    /// Moves `entity` into `archetype`, carrying the values both archetypes
    /// hold and destroying the ones it is losing.
    ///
    /// Returns the row it landed in — **mid-write** for any component the
    /// destination has and the source did not, which the caller fills.
    ///
    /// 🔴 This is where the displaced entity gets fixed. A row move pulls
    /// the last row of the source table into the hole, so a **second**
    /// entity — one that asked for nothing and changed no components —
    /// lands somewhere new. Its row is updated here, because nowhere else
    /// knows it happened.
    pub fn relocate(
        &mut self,
        entity: Entity,
        archetype: ArchetypeId,
        components: &ComponentRegistry,
    ) -> Option<TableRow> {
        let Some(row) = self.row_of(entity) else {
            return self.place(entity, archetype, components);
        };
        let from = self.entity_archetype.get(&entity).copied()?;
        let source = self.table_of(from, components)?;
        let target = self.table_of(archetype, components)?;

        if source == target {
            // The same set of stored components: the row does not move,
            // only the archetype the entity is filed under. Today that can
            // only be the archetype it already had; it becomes reachable
            // when a component opts out of table storage.
            self.register_entity(entity, archetype);
            return Some(row);
        }

        let (landed, displaced) = self.tables.move_row(source, row, target);
        if let Some(other) = displaced {
            self.entity_rows.insert(other, row);
        }
        self.register_entity(entity, archetype);
        self.entity_rows.insert(entity, landed);
        Some(landed)
    }

    /// Removes `entity` from its table, destroying its values, and from the
    /// archetype index.
    ///
    /// Returns `false` if it held no row. The entity displaced by the
    /// removal is fixed here, for the same reason as in [`Self::relocate`].
    pub fn evict(&mut self, entity: Entity, components: &ComponentRegistry) -> bool {
        let Some(row) = self.entity_rows.remove(&entity) else {
            self.unregister_entity(entity);
            return false;
        };
        let Some(archetype) = self.entity_archetype.get(&entity).copied() else {
            return false;
        };
        let Some(table) = self.table_of(archetype, components) else {
            return false;
        };
        let displaced = self
            .tables
            .get_mut(table)
            .and_then(|table| table.swap_remove(row));
        if let Some(other) = displaced {
            self.entity_rows.insert(other, row);
        }
        self.unregister_entity(entity);
        true
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
