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
        }
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
        let slot = self.slot(type_id)?;
        let accessor = slot.reflector.as_deref()?;
        // SAFETY: We have &self and no mutable references are active.
        let storage_ref = unsafe { &**slot.storage.get() };
        accessor.get_fields(storage_ref, entity)
    }

    /// Sets a single reflected field on a component for an entity.
    pub fn reflect_set_field(
        &mut self,
        type_id: &TypeId,
        entity: Entity,
        field: &str,
        value: ReflectValue,
    ) -> Result<(), ReflectError> {
        let slot = self.slot(type_id).ok_or(ReflectError::ComponentNotFound)?;
        let accessor = slot
            .reflector
            .as_deref()
            .ok_or(ReflectError::ComponentNotFound)?;
        // SAFETY: We have &mut self, so exclusive access is guaranteed.
        let storage_mut = unsafe { &mut **slot.storage.get() };
        accessor.set_field(storage_mut, entity, field, value)
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

    /// Returns a mutable reference to the type-erased storage for the given `TypeId`.
    ///
    /// # Safety
    ///
    /// Caller must ensure no other reference (mutable or immutable) to the
    /// same `TypeId` storage is active.
    pub(crate) unsafe fn storage_mut(&self, type_id: &TypeId) -> Option<&mut dyn AnyStorage> {
        let slot = self.slot(type_id)?;
        Some(unsafe { &mut **slot.storage.get() })
    }
}

impl Default for ComponentRegistry {
    fn default() -> Self {
        Self::new()
    }
}
