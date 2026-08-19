//! Core [`ComponentRegistry`] type and inherent impls.
//!
//! # Why one dense slot instead of a map per concern
//!
//! Storages, type names and reflectors used to be three `HashMap`s keyed
//! by the same `TypeId`: three lookups where one is enough, and three
//! chances for a type to be present in one and absent from another.
//!
//! 🔴 The reason it changed is not tidiness. A **column cannot be indexed
//! by a `TypeId`** — it is a 128-bit hash, not an integer. Dense component
//! columns are what #891 is for, and they need an integer handle that
//! exists before the columns do.
//!
//! So the registry mints a [`StorageId`] on first registration and keeps
//! one slot per id. The `TypeId` map survives for exactly one job: turning
//! a Rust type into its id. That happens at registration and when a query
//! is built — **never per entity**, which is the whole point.

use std::any::TypeId;
use std::cell::UnsafeCell;
use std::collections::HashMap;

use crate::entity::Entity;
use crate::reflect::{
    FieldMeta, InspectorVisibility, Reflect, ReflectAccessor, ReflectError, ReflectValue,
    TypedReflectAccessor,
};

use crate::component::cpu_storage::ComponentStorage;
use crate::component::storage_id::StorageId;
use crate::component::traits::{AnyStorage, Component};
use crate::storage::Column;
use crate::storage::{Table, TableId, TableRow, Tables};

/// Everything the registry knows about one registered component type.
///
/// One slot, not four parallel arrays: these four are written together at
/// registration and read together afterwards, so splitting them would buy
/// nothing and cost a way for them to disagree.
pub(super) struct Slot {
    storage: UnsafeCell<Box<dyn AnyStorage>>,
    type_id: TypeId,
    name: &'static str,
    /// `None` for a component registered without reflection.
    reflector: Option<Box<dyn ReflectAccessor>>,
    /// Builds an empty column for this component's type.
    ///
    /// A monomorphised function pointer, captured at registration: it is
    /// the only place the concrete type is still in scope, and a table
    /// built later has nothing but the id.
    new_column: fn() -> Column,
}

/// Central registry for all component storages.
///
/// Holds one [`ComponentStorage<T>`] per registered component type, in a
/// dense slot addressed by [`StorageId`].
///
/// Uses [`UnsafeCell`] internally to allow the query system to borrow
/// multiple storages simultaneously. Safety is enforced at the query
/// level through runtime access tracking.
pub struct ComponentRegistry {
    /// Indexed by [`StorageId`]. Append-only: a slot is never removed, so
    /// an id handed out stays valid for the life of the registry.
    pub(super) slots: Vec<Slot>,
    /// The only map left, and it is consulted at registration and at query
    /// construction — never inside a walk.
    pub(super) ids: HashMap<TypeId, StorageId>,
    /// Where component VALUES live, for the ones that have moved (#891).
    ///
    /// 🎯 Here and not on the archetype registry, and the split is the
    /// point: an archetype is an **index** — who has which components —
    /// while a table is **where the value is**. Reflection and queries both
    /// already ask this registry for a component; with the tables here they
    /// can resolve a location without reaching for a second registry, which
    /// is what would otherwise have meant threading one through eighty call
    /// sites in three crates.
    ///
    /// It is also how Bevy splits it: `Components`/`Storages` against
    /// `Archetypes`.
    pub(super) tables: Tables,
    /// Which table row holds an entity's values, once they live in one.
    pub(super) entity_at: HashMap<Entity, (TableId, TableRow)>,
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
            entity_at: HashMap::new(),
        }
    }

    // -- Where the values live (#891) -----------------------------------------

    /// The tables holding component values.
    #[inline]
    pub fn tables(&self) -> &Tables {
        &self.tables
    }

    /// The tables, mutably.
    #[inline]
    pub fn tables_mut(&mut self) -> &mut Tables {
        &mut self.tables
    }

    /// Where `entity`'s values live, if they live in a table.
    #[inline]
    pub fn location(&self, entity: Entity) -> Option<(TableId, TableRow)> {
        self.entity_at.get(&entity).copied()
    }

    /// The table serving `components`, **without creating one**.
    #[inline]
    pub fn table_for(&self, components: &[StorageId]) -> Option<TableId> {
        self.tables.find(components)
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
    ///
    /// The registry is the only thing that still knows the concrete type
    /// by the time a table needs a column, so it is the only thing that
    /// can build one.
    pub fn new_column(&self, id: StorageId) -> Option<Column> {
        Some((self.slots.get(id.index())?.new_column)())
    }

    #[inline]
    fn slot(&self, type_id: &TypeId) -> Option<&Slot> {
        self.slots.get(self.ids.get(type_id)?.index())
    }

    /// Registers a CPU-only component type.
    ///
    /// Does nothing if the type is already registered, and returns the
    /// existing id in that case — registration is idempotent and an id,
    /// once handed out, never moves.
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
        });
        self.ids.insert(type_id, id);
        id
    }

    /// Returns an immutable reference to a CPU component storage.
    pub fn get_cpu<T: Component>(&self) -> Option<&ComponentStorage<T>> {
        let slot = self.slot(&TypeId::of::<T>())?;
        // SAFETY: No mutable references exist (we have &self, not via query).
        let storage = unsafe { &*slot.storage.get() };
        storage.as_any().downcast_ref()
    }

    /// Returns a mutable reference to a CPU component storage.
    pub fn get_cpu_mut<T: Component>(&mut self) -> Option<&mut ComponentStorage<T>> {
        let id = self.ids.get(&TypeId::of::<T>()).copied()?;
        let slot = self.slots.get_mut(id.index())?;
        // SAFETY: We have &mut self, so exclusive access is guaranteed.
        slot.storage.get_mut().as_any_mut().downcast_mut()
    }

    /// Removes `entity` from all registered storages.
    pub fn remove_entity(&mut self, entity: Entity) {
        for slot in &mut self.slots {
            slot.storage.get_mut().remove_entity(entity);
        }
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
    ///
    /// The component must implement both [`Component`] and [`Reflect`].
    /// Keeps the existing reflector if the type already has one.
    pub fn register_cpu_reflected<T: Component + Reflect>(&mut self) {
        let id = self.register_cpu::<T>();
        let slot = &mut self.slots[id.index()];
        if slot.reflector.is_none() {
            slot.reflector = Some(Box::new(TypedReflectAccessor::<T>::new_cpu()));
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
    ///
    /// No entity involved: this is for building a component somewhere an
    /// entity does not exist, such as adding one to a prefab document.
    pub fn reflect_default_fields(&self, type_id: &TypeId) -> Option<Vec<(String, ReflectValue)>> {
        Some(self.reflector(type_id)?.default_fields())
    }

    /// Reads all reflected field values for a component on an entity.
    ///
    /// Returns `None` if the component type has no reflector, no storage,
    /// or the entity does not have the component.
    pub fn reflect_get_fields(
        &self,
        type_id: &TypeId,
        entity: Entity,
    ) -> Option<Vec<(String, ReflectValue)>> {
        self.reflect_fields_at(type_id, entity, self.at(entity))
    }

    /// Reads a component's fields from wherever its value actually lives.
    ///
    /// 🔴 The inspector, scene saving, undo, prefabs and the remote mirror
    /// all reach components through here rather than through a query, so
    /// this is the one place that has to know about both homes. A value
    /// that moved to a column and was only readable by queries would show
    /// as an empty component in the inspector and be lost by a save — with
    /// nothing failing. See #891.
    ///
    /// `at` is `None` for a component still held in the per-type map,
    /// which today is all of them.
    pub fn reflect_fields_at(
        &self,
        type_id: &TypeId,
        entity: Entity,
        at: Option<(&Table, TableRow)>,
    ) -> Option<Vec<(String, ReflectValue)>> {
        let slot = self.slot(type_id)?;
        let accessor = slot.reflector.as_deref()?;
        let id = self.ids.get(type_id).copied();

        if let Some((table, row)) = at
            && let Some(column) = id.and_then(|id| table.column(id))
        {
            // SAFETY: the column was built for this component's type, and
            // the row is the caller's to have checked.
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
        // 🔴 The value's ADDRESS first, so every borrow it needed is
        // released before the accessor is reached. Handing a `&Table` down
        // while `&mut self` is live is an aliasing violation — and laundering
        // it through a raw pointer only hides it from the compiler, not from
        // Miri.
        let slot = *self
            .ids
            .get(type_id)
            .ok_or(ReflectError::ComponentNotFound)?;
        let column_value = self
            .entity_at
            .get(&entity)
            .copied()
            .and_then(|(table, row)| {
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

        let accessor = self.slots[slot.index()]
            .reflector
            .as_deref()
            .ok_or(ReflectError::ComponentNotFound)?;
        // SAFETY: `target` points at a live component of this type, and the
        // borrow above is the exclusive access.
        unsafe { accessor.write_field(target, field, value) }
    }

    /// Sets a field on a component wherever its value actually lives.
    ///
    /// The write side of [`Self::reflect_fields_at`], and it has to follow
    /// the same rule: reading from one home and writing to the other would
    /// let the inspector show a value it cannot change.
    pub fn reflect_write_at(
        &mut self,
        type_id: &TypeId,
        entity: Entity,
        field: &str,
        value: ReflectValue,
        at: Option<(&Table, TableRow)>,
    ) -> Result<(), ReflectError> {
        let id = self.ids.get(type_id).copied();
        let slot = self.slot(type_id).ok_or(ReflectError::ComponentNotFound)?;
        let accessor = slot
            .reflector
            .as_deref()
            .ok_or(ReflectError::ComponentNotFound)?;

        if let Some((table, row)) = at
            && let Some(column) = id.and_then(|id| table.column(id))
        {
            // SAFETY: the column holds this component's type, and `&mut
            // self` is the exclusive access the write needs.
            let target = unsafe { column.value_ptr::<u8>(row.index()) }
                .ok_or(ReflectError::ComponentNotFound)?;
            return unsafe { accessor.write_field(target, field, value) };
        }

        // SAFETY: We have &mut self, so exclusive access is guaranteed.
        let storage = unsafe { &mut **slot.storage.get() };
        let target = storage
            .get_mut_ptr(entity)
            .ok_or(ReflectError::ComponentNotFound)?;
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

    /// Looks up a `TypeId` by its full type name string.
    ///
    /// Linear scan of the slots. Only called at scene load time.
    pub fn type_id_by_name(&self, name: &str) -> Option<TypeId> {
        self.slots
            .iter()
            .find(|slot| slot.name == name)
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
    ///
    /// Returns `true` if the component was inserted successfully.
    /// Returns `false` if the type has no reflector, no storage, or insert failed.
    pub fn insert_default_reflected(&mut self, type_id: &TypeId, entity: Entity) -> bool {
        let Some(slot) = self.slot(type_id) else {
            return false;
        };
        let Some(accessor) = slot.reflector.as_deref() else {
            return false;
        };
        // SAFETY: We have &mut self, exclusive access guaranteed.
        let storage = unsafe { &mut **slot.storage.get() };
        accessor.insert_default_into(storage, entity)
    }

    // -- Type-erased storage access -------------------------------------------

    /// Returns an immutable reference to the type-erased storage for the given `TypeId`.
    ///
    /// # Safety
    ///
    /// Caller must ensure no mutable reference to the same `TypeId` storage
    /// is active (i.e. no concurrent `storage_mut` call for the same type).
    pub(crate) unsafe fn storage(&self, type_id: &TypeId) -> Option<&dyn AnyStorage> {
        let slot = self.slot(type_id)?;
        Some(unsafe { &**slot.storage.get() })
    }

    /// A **raw pointer** to the type-erased storage, for writing.
    ///
    /// 🔴 A pointer and not a `&mut`, and the difference is soundness. A
    /// `&mut` stored in a fetch and later reborrowed **shared** — which is
    /// what `fetch(&self, …)` does — can only be cast back to `*mut` by
    /// retagging a shared tag for writes. Stacked Borrows forbids that, and
    /// Miri caught it doing exactly that on a test that had been passing.
    ///
    /// The pointer here comes straight from the `UnsafeCell`, so its
    /// provenance grants writes for as long as the registry lives.
    ///
    /// # Safety
    ///
    /// Caller must ensure no other reference to the same storage is active.
    /// The query system's borrow tracker is what does that upstream.
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
