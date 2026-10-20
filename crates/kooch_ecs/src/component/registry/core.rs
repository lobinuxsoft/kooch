//! Core [`ComponentRegistry`] type and inherent impls.

use std::any::TypeId;
use std::cell::UnsafeCell;
use std::collections::HashMap;

use crate::entity::Entity;
use crate::reflect::{
    FieldMeta, InspectorVisibility, Reflect, ReflectAccessor, ReflectError, ReflectValue,
    TypedReflectAccessor,
};

use crate::component::cpu_storage::ComponentStorage;
use crate::component::locations::Locations;
use crate::component::storage_id::StorageId;
use crate::component::traits::{AnyStorage, Component};
use crate::component::view::{ComponentView, ComponentViewMut};
use crate::storage::Column;
use crate::storage::{Table, TableId, TableRow, Tables};

/// Where an entity ended up after a move, and whoever its move disturbed.
#[derive(Debug, Clone, Copy, Default)]
pub struct Settled {
    /// The row holding its values, or `None` when they are still in the inbox.
    pub row: Option<TableRow>,
    /// The entity dragged into the hole the move left. Its row changed and it asked for nothing.
    pub displaced: Option<Entity>,
}

/// Everything the registry knows about one registered component type.
pub(super) struct Slot {
    storage: UnsafeCell<Box<dyn AnyStorage>>,
    type_id: TypeId,
    name: &'static str,
    /// `None` for a component registered without reflection.
    reflector: Option<Box<dyn ReflectAccessor>>,
    /// Builds an empty column for this component's type.
    new_column: fn() -> Column,
    /// Bumped by every write to this component, wherever the value lives.
    ///
    /// 🔴 On the slot, not on the map. The editor's frame snapshot cache keys on it (#1561, the
    /// 29 → 140 FPS change) and the values moved to the columns — a revision left behind on a map
    /// nobody writes to any more does not fail, it stops being a signal, in silence.
    revision: u64,
}

/// Central registry for all component storages.
pub struct ComponentRegistry {
    /// Indexed by [`StorageId`]. Append-only: a slot is never removed, so
    /// an id handed out stays valid for the life of the registry.
    pub(super) slots: Vec<Slot>,
    /// The only map left, and it is consulted at registration and at query
    /// construction — never inside a walk.
    pub(super) ids: HashMap<TypeId, StorageId>,
    /// Where component VALUES live, for the ones that have moved (#891).
    pub(super) tables: Tables,
    /// Which table row holds an entity's values, once they live in one.
    pub(super) entity_at: Locations,
}

// SAFETY: All public methods either take `&mut self` (exclusive access) or
// `&self` for read-only operations. The query system uses `UnsafeCell` access
// with runtime borrow checking to ensure no aliasing violations.
unsafe impl Send for ComponentRegistry {}
unsafe impl Sync for ComponentRegistry {}

impl ComponentRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            ids: HashMap::new(),
            tables: Tables::new(),
            entity_at: Locations::default(),
        }
    }

    // -- Where the values live (#891) -----------------------------------------

    /// The tables holding component values.
    #[inline]
    pub fn tables(&self) -> &Tables {
        &self.tables
    }

    #[cfg(test)]
    /// The tables, mutably.
    #[inline]
    pub fn tables_mut(&mut self) -> &mut Tables {
        &mut self.tables
    }

    /// Where `entity`'s values live, if they live in a table.
    #[inline]
    pub fn location(&self, entity: Entity) -> Option<(TableId, TableRow)> {
        self.entity_at.get(entity)
    }

    /// The table serving `components`, **without creating one**.
    #[inline]
    pub fn table_for(&self, components: &[StorageId]) -> Option<TableId> {
        self.tables.find(components)
    }

    /// The table serving `components`, built on first ask.
    ///
    /// The tables come out of `self` for the call: building a column needs the registry that knows
    /// how, and that is the same `self` the tables live in.
    pub fn table_of(&mut self, components: &[StorageId]) -> TableId {
        let mut tables = std::mem::take(&mut self.tables);
        let table = tables.get_or_insert(self, components);
        self.tables = tables;
        table
    }

    /// How many times the component `id` names has been written to.
    #[inline]
    pub fn slot_revision(&self, id: StorageId) -> u64 {
        self.slots.get(id.index()).map_or(0, |slot| slot.revision)
    }

    /// Records a write to the component `id` names.
    #[inline]
    pub fn touch(&mut self, id: StorageId) {
        if let Some(slot) = self.slots.get_mut(id.index()) {
            slot.revision += 1;
        }
    }

    /// Settles `entity` into the table serving `components`, promoting every value still parked in
    /// a per-type map into the column that now holds it.
    ///
    /// Reports the entity dragged into the hole this move left, because its row changed and it
    /// asked for nothing. 🔴 The return type is the guard: a caller cannot forget what it is
    /// handed, and an un-refiled entity reads another entity's components with nothing said.
    ///
    /// 🔴 All of it or none of it. A row that moves while one value stays behind leaves a column
    /// short; an archetype that changes while the row stays behind leaves the entity reading
    /// **another entity's components**, with nothing said. So a move that cannot be completed is
    /// undone entirely: the values go back to the maps, the row is dropped, and the dual read in
    /// `fetch` keeps serving the entity until the missing value shows up.
    ///
    /// Returns the row the entity ended up in, or `None` when it has none.
    pub fn settle(&mut self, entity: Entity, components: &[StorageId]) -> Settled {
        if components.is_empty() || !self.values_are_all_in(entity, components) {
            // 🔴 `components` and not everything: a value whose component the target does not have
            // is one the entity is **losing**, and handing that back to the inbox would resurrect
            // the component the caller just took away.
            return Settled {
                row: None,
                displaced: self.unsettle_but(entity, components),
            };
        }

        let (row, displaced) = self.relocate(entity, components);
        let Some(row) = row else {
            return Settled {
                row: None,
                displaced,
            };
        };
        for id in components {
            self.promote(entity, *id, row);
        }
        debug_assert!(
            self.location(entity)
                .and_then(|(table, _)| self.tables.get(table))
                .is_none_or(|table| table.rows_agree()),
            "a column drifted out of step"
        );
        Settled {
            row: Some(row),
            displaced,
        }
    }

    /// Whether every one of `components` has a value for `entity` — already in its column, or
    /// parked in its map waiting to be promoted.
    fn values_are_all_in(&self, entity: Entity, components: &[StorageId]) -> bool {
        let held = self
            .location(entity)
            .and_then(|(table, _)| self.tables.get(table));
        components.iter().all(|id| {
            if held.is_some_and(|table| table.column(*id).is_some()) {
                return true;
            }
            self.slots.get(id.index()).is_some_and(|slot| {
                // SAFETY: `&self` is the shared access a read needs, and no
                // query holds a mutable borrow while the funnel runs.
                unsafe { &**slot.storage.get() }.holds(entity)
            })
        })
    }

    /// Moves `entity`'s value for `id` out of its map and into the column at `row`.
    ///
    /// A no-op when the value is already in the column, which is the common case: only a component
    /// the entity just gained needs promoting.
    fn promote(&mut self, entity: Entity, id: StorageId, row: TableRow) {
        let Some((table, _)) = self.location(entity) else {
            return;
        };
        let Some(held) = self.tables.get_mut(table) else {
            return;
        };
        let Some(column) = held.column_mut(id) else {
            return;
        };
        if column.len() > row.index() {
            return;
        }
        let Some(slot) = self.slots.get_mut(id.index()) else {
            return;
        };
        slot.storage.get_mut().take_into(entity, column);
    }

    /// Puts every one of `entity`'s values back into its inbox and drops its row.
    ///
    /// The way out of a move that cannot finish: no table can serve the archetype, so the values
    /// go back where the dual read in `fetch` can still find them.
    pub fn unsettle(&mut self, entity: Entity) -> Option<Entity> {
        let keep: Vec<StorageId> = (0..self.slots.len() as u32).map(StorageId).collect();
        self.unsettle_but(entity, &keep)
    }

    /// The same, except values whose component is not in `keep` are **destroyed**.
    ///
    /// That is the difference between a move that could not finish and a component the entity gave
    /// up: the first keeps its values, the second must not.
    pub fn unsettle_but(&mut self, entity: Entity, keep: &[StorageId]) -> Option<Entity> {
        let (table, row) = self.entity_at.take(entity)?;
        let held = self.tables.get_mut(table)?;
        // The slots are indexed by `StorageId`, so the drain can reach each one by index without
        // borrowing the whole registry — which the closure cannot do while the table is borrowed.
        let slots = std::ptr::from_mut(&mut self.slots);
        let displaced = held.drain_row(row, |id, column, at| {
            if !keep.contains(&id) {
                column.swap_remove(at);
                return;
            }
            // SAFETY: `slots` and `self.tables` are distinct fields, and the
            // closure touches only the former.
            if let Some(slot) = unsafe { &mut *slots }.get_mut(id.index()) {
                slot.storage.get_mut().put_from(entity, column, at);
            }
        });
        if let Some(other) = displaced {
            self.entity_at.set(other, table, row);
        }
        displaced
    }

    /// Gives `entity` a row in the table serving `components`, with its columns one short — the
    /// caller promotes the values into them.
    fn place(&mut self, entity: Entity, components: &[StorageId]) -> Option<TableRow> {
        let table = self.table_of(components);
        let row = self.tables.get_mut(table)?.push_entity(entity);
        self.entity_at.set(entity, table, row);
        Some(row)
    }

    /// Moves `entity` into the table serving `components`, carrying the values both tables hold and
    /// destroying the ones it is losing.
    ///
    /// 🔴 A row that moves drags the last row into the hole it left, so the entity that was there
    /// is re-filed here. Miss that and it reads another entity's components, silently.
    fn relocate(
        &mut self,
        entity: Entity,
        components: &[StorageId],
    ) -> (Option<TableRow>, Option<Entity>) {
        let Some((source, row)) = self.location(entity) else {
            return (self.place(entity, components), None);
        };
        let target = self.table_of(components);
        if source == target {
            return (Some(row), None);
        }

        let (landed, displaced) = self.tables.move_row(source, row, target);
        if let Some(other) = displaced {
            self.entity_at.set(other, source, row);
        }
        self.entity_at.set(entity, target, landed);
        (Some(landed), displaced)
    }

    /// The table and row holding `entity`'s values, resolved for reading.
    fn at(&self, entity: Entity) -> Option<(&Table, TableRow)> {
        let (table, row) = self.location(entity)?;
        Some((self.tables.get(table)?, row))
    }

    // -- The dense handle -----------------------------------------------------

    /// The slot `type_id` was registered into, if it was.
    ///
    /// Resolve once and keep the id: that is what makes the walk free.
    pub fn storage_id(&self, type_id: &TypeId) -> Option<StorageId> {
        self.ids.get(type_id).copied()
    }

    /// How many component types are registered.
    ///
    /// Also the exclusive upper bound of every live [`StorageId`].
    pub fn registered_count(&self) -> usize {
        self.slots.len()
    }

    /// A fresh, empty column for the component `id` names.
    pub fn new_column(&self, id: StorageId) -> Option<Column> {
        Some((self.slots.get(id.index())?.new_column)())
    }

    #[inline]
    fn slot(&self, type_id: &TypeId) -> Option<&Slot> {
        self.slots.get(self.ids.get(type_id)?.index())
    }

    /// Registers a CPU-only component type.
    pub fn register_cpu<T: Component>(&mut self) -> StorageId {
        let type_id = TypeId::of::<T>();
        if let Some(id) = self.ids.get(&type_id) {
            return *id;
        }
        let id = StorageId(self.slots.len() as u32);
        self.slots.push(Slot {
            storage: UnsafeCell::new(Box::new(ComponentStorage::<T>::new())),
            type_id,
            name: std::any::type_name::<T>(),
            reflector: None,
            new_column: Column::of::<T>,
            revision: 0,
        });
        self.ids.insert(type_id, id);
        id
    }

    /// A read-only view of one component type, wherever its values live.
    ///
    /// 🔴 Not `&ComponentStorage<T>` any more (#891). A value lives in exactly one place and most
    /// of them have moved into a table column, so a map reference would read `None` for an entity
    /// that plainly has the component — silently, at six hundred call sites.
    pub fn get_cpu<T: Component>(&self) -> Option<ComponentView<'_, T>> {
        let id = self.ids.get(&TypeId::of::<T>()).copied()?;
        let slot = self.slots.get(id.index())?;
        // SAFETY: No mutable references exist (we have &self, not via query).
        let map = unsafe { &*slot.storage.get() }.as_any().downcast_ref()?;
        Some(ComponentView {
            id,
            map,
            tables: &self.tables,
            entity_at: &self.entity_at,
            revision: slot.revision,
        })
    }

    /// A writable view of one component type.
    pub fn get_cpu_mut<T: Component>(&mut self) -> Option<ComponentViewMut<'_, T>> {
        let id = self.ids.get(&TypeId::of::<T>()).copied()?;
        let slot = self.slots.get_mut(id.index())?;
        // Three disjoint fields: the slot's storage, the slot's revision, and the tables. Splitting
        // them here is what lets one view both read a column and park a value in the map.
        let revision = &mut slot.revision;
        let map = slot.storage.get_mut().as_any_mut().downcast_mut()?;
        Some(ComponentViewMut {
            id,
            map,
            tables: &mut self.tables,
            entity_at: &self.entity_at,
            revision,
        })
    }

    /// Removes `entity` from all registered storages, columns included.
    pub fn remove_entity(&mut self, entity: Entity) {
        self.drop_row(entity);
        for slot in &mut self.slots {
            slot.storage.get_mut().remove_entity(entity);
            slot.revision += 1;
        }
    }

    /// Frees `entity`'s row and destroys the values in it.
    ///
    /// Unlike [`Self::unsettle`] the values are **not** handed back to the maps: this is a despawn,
    /// and nothing is going to read them again.
    pub fn drop_row(&mut self, entity: Entity) -> Option<Entity> {
        let (table, row) = self.entity_at.take(entity)?;
        let displaced = self.tables.get_mut(table)?.swap_remove(row);
        if let Some(other) = displaced {
            self.entity_at.set(other, table, row);
        }
        displaced
    }

    /// Removes a single component from `entity` by `TypeId`.
    ///
    /// Does nothing if the type is not registered or the entity doesn't have it.
    pub fn remove_component(&mut self, entity: Entity, type_id: &TypeId) {
        let Some(id) = self.ids.get(type_id).copied() else {
            return;
        };
        if let Some(slot) = self.slots.get_mut(id.index()) {
            slot.storage.get_mut().remove_entity(entity);
            slot.revision += 1;
        }
    }

    /// Returns `true` if a storage is registered for the given `TypeId`.
    pub fn contains_type(&self, type_id: &TypeId) -> bool {
        self.ids.contains_key(type_id)
    }

    /// Returns the human-readable type name for a registered component.
    pub fn component_name(&self, type_id: &TypeId) -> Option<&'static str> {
        self.slot(type_id).map(|slot| slot.name)
    }

    // -- Reflected registration -------------------------------------------------

    /// Registers a CPU-only component with reflection support.
    pub fn register_cpu_reflected<T: Component + Reflect>(&mut self) {
        let id = self.register_cpu::<T>();
        self.refuse_claimed_names::<T>(id);
        let slot = &mut self.slots[id.index()];
        if slot.reflector.is_none() {
            slot.reflector = Some(Box::new(TypedReflectAccessor::<T>::new_cpu()));
        }
    }

    /// Panics where `T`'s name or one of its aliases is already another type's.
    ///
    /// 🔴 Loudly, at registration, because the alternative is a load picking one of them and the
    /// other component vanishing from every entity with nothing said (#1368). Both directions: the
    /// clash is the same whichever of the two registered first.
    fn refuse_claimed_names<T: Component + Reflect>(&self, mine: StorageId) {
        let name = std::any::type_name::<T>();
        for alias in T::aliases() {
            if let Some(other) = self.slots.iter().find(|slot| slot.name == *alias) {
                assert!(
                    other.type_id == TypeId::of::<T>(),
                    "{name} claims the alias {alias}, which is {}'s own name",
                    other.name,
                );
            }
        }
        for (at, slot) in self.slots.iter().enumerate() {
            if at == mine.index() {
                continue;
            }
            let taken = slot
                .reflector
                .as_ref()
                .is_some_and(|reflector| reflector.aliases().contains(&name));
            assert!(
                !taken,
                "{} claims the alias {name}, which is a live type",
                slot.name
            );
        }
    }

    // -- Reflection API -------------------------------------------------------

    #[inline]
    fn reflector(&self, type_id: &TypeId) -> Option<&dyn ReflectAccessor> {
        self.slot(type_id)?.reflector.as_deref()
    }

    /// Returns `true` if a reflection accessor is registered for `type_id`.
    pub fn has_reflector(&self, type_id: &TypeId) -> bool {
        self.reflector(type_id).is_some()
    }

    /// Returns field metadata for a reflected component type.
    pub fn reflect_field_metas(&self, type_id: &TypeId) -> Option<&'static [FieldMeta]> {
        self.reflector(type_id).map(|r| r.fields())
    }

    /// The field values a freshly-constructed component would have.
    pub fn reflect_default_fields(&self, type_id: &TypeId) -> Option<Vec<(String, ReflectValue)>> {
        Some(self.reflector(type_id)?.default_fields())
    }

    /// Reads all reflected field values for a component on an entity.
    pub fn reflect_get_fields(
        &self,
        type_id: &TypeId,
        entity: Entity,
    ) -> Option<Vec<(String, ReflectValue)>> {
        let slot = self.slot(type_id)?;
        let accessor = slot.reflector.as_deref()?;

        if let Some(id) = self.ids.get(type_id).copied()
            && let Some((table, row)) = self.at(entity)
            && let Some(column) = table.column(id)
        {
            // SAFETY: the column was built for this component's type, and
            // the row came from this registry's own index.
            let value = unsafe { column.value_ptr::<u8>(row.index())? };
            return Some(unsafe { accessor.read_fields(value) });
        }

        // SAFETY: We have &self and no mutable references are active.
        let storage = unsafe { &**slot.storage.get() };
        let value = storage.get_ptr(entity)?;
        Some(unsafe { accessor.read_fields(value) })
    }

    /// Sets a single reflected field on a component for an entity.
    pub fn reflect_set_field(
        &mut self,
        type_id: &TypeId,
        entity: Entity,
        field: &str,
        value: ReflectValue,
    ) -> Result<(), ReflectError> {
        // 🔴 The value's ADDRESS first, so every borrow it needed is released before the accessor is
        // reached. Handing a `&Table` down while `&mut self` is live is an aliasing violation — and
        // laundering it through a raw pointer only hides it from the compiler, not from Miri.
        let slot = *self
            .ids
            .get(type_id)
            .ok_or(ReflectError::ComponentNotFound)?;
        let column_value = self.entity_at.get(entity).and_then(|(table, row)| {
            let column = self.tables.get(table)?.column(slot)?;
            // SAFETY: the column holds this component's type, and the
            // pointer derives from the column's own allocation.
            unsafe { column.value_ptr::<u8>(row.index()) }
        });

        let target = match column_value {
            Some(value) => value,
            None => {
                // SAFETY: `&mut self` is the exclusive access the write needs.
                let storage = unsafe { &mut **self.slots[slot.index()].storage.get() };
                storage
                    .get_mut_ptr(entity)
                    .ok_or(ReflectError::ComponentNotFound)?
            }
        };

        self.slots[slot.index()].revision += 1;
        let accessor = self.slots[slot.index()]
            .reflector
            .as_deref()
            .ok_or(ReflectError::ComponentNotFound)?;
        // SAFETY: `target` points at a live component of this type, and the
        // borrow above is the exclusive access.
        unsafe { accessor.write_field(target, field, value) }
    }

    /// Returns the inspector visibility for a reflected component type.
    pub fn reflect_inspector_visibility(&self, type_id: &TypeId) -> Option<InspectorVisibility> {
        self.reflector(type_id).map(|r| r.inspector_visibility())
    }

    /// Returns the editor category for a reflected component type, if any.
    pub fn reflect_category(&self, type_id: &TypeId) -> Option<&'static str> {
        self.reflector(type_id).and_then(|r| r.category())
    }

    /// Returns all `TypeId`s that have a registered reflector.
    pub fn reflected_type_ids(&self) -> Vec<TypeId> {
        self.slots
            .iter()
            .filter(|slot| slot.reflector.is_some())
            .map(|slot| slot.type_id)
            .collect()
    }

    /// Looks up a `TypeId` by its full type name string, or by a name the type used to be saved
    /// under (#1368).
    ///
    /// Linear scan of the slots. Only called at scene load time.
    ///
    /// 🔴 Live names first, every one of them, before any alias is considered. A type that still
    /// exists always answers for its own name, whatever an unrelated type claims it used to be
    /// called — otherwise registration order would decide which component a scene gets.
    pub fn type_id_by_name(&self, name: &str) -> Option<TypeId> {
        self.slots
            .iter()
            .find(|slot| slot.name == name)
            .or_else(|| {
                self.slots.iter().find(|slot| {
                    slot.reflector
                        .as_ref()
                        .is_some_and(|reflector| reflector.aliases().contains(&name))
                })
            })
            .map(|slot| slot.type_id)
    }

    /// Returns all registered component types with their human-readable names.
    pub fn all_type_names(&self) -> Vec<(TypeId, &'static str)> {
        self.slots
            .iter()
            .map(|slot| (slot.type_id, slot.name))
            .collect()
    }

    /// Returns all reflected types with their human-readable names.
    pub fn reflected_type_names(&self) -> Vec<(TypeId, &'static str)> {
        self.slots
            .iter()
            .filter(|slot| slot.reflector.is_some())
            .map(|slot| (slot.type_id, slot.name))
            .collect()
    }

    /// Inserts a default reflected component for an entity.
    pub fn insert_default_reflected(&mut self, type_id: &TypeId, entity: Entity) -> bool {
        let Some(slot) = self.slot(type_id) else {
            return false;
        };
        let Some(accessor) = slot.reflector.as_deref() else {
            return false;
        };
        // SAFETY: We have &mut self, exclusive access guaranteed.
        let storage = unsafe { &mut **slot.storage.get() };
        let inserted = accessor.insert_default_into(storage, entity);
        if inserted && let Some(id) = self.ids.get(type_id).copied() {
            self.slots[id.index()].revision += 1;
        }
        inserted
    }

    // -- Type-erased storage access -------------------------------------------

    /// Returns an immutable reference to the type-erased storage for the given `TypeId`.
    pub(crate) unsafe fn storage(&self, type_id: &TypeId) -> Option<&dyn AnyStorage> {
        let slot = self.slot(type_id)?;
        Some(unsafe { &**slot.storage.get() })
    }

    /// A **raw pointer** to the type-erased storage, for writing.
    pub(crate) unsafe fn storage_ptr(&self, type_id: &TypeId) -> Option<*mut dyn AnyStorage> {
        let slot = self.slot(type_id)?;
        Some(unsafe { &mut **slot.storage.get() as *mut dyn AnyStorage })
    }
}

impl Default for ComponentRegistry {
    fn default() -> Self {
        Self::new()
    }
}
