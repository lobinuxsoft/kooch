//! Core [`ComponentRegistry`] type and inherent impls.

use std::any::TypeId;
use std::cell::UnsafeCell;
use std::collections::HashMap;

use wgpu::{Device, Queue};

use crate::entity::Entity;
use crate::reflect::{
    FieldMeta, InspectorVisibility, Reflect, ReflectAccessor, ReflectError, ReflectValue,
    TypedReflectAccessor,
};

use crate::component::cpu_storage::ComponentStorage;
use crate::component::traits::{AnyStorage, Component};

/// Central registry for all component storages.
///
/// Stores one [`ComponentStorage<T>`] per
/// registered component type, keyed by `TypeId`.
///
/// Uses [`UnsafeCell`] internally to allow the query system to borrow
/// multiple storages simultaneously. Safety is enforced at the query
/// level through runtime access tracking.
pub struct ComponentRegistry {
    pub(super) storages: HashMap<TypeId, UnsafeCell<Box<dyn AnyStorage>>>,
    pub(super) type_names: HashMap<TypeId, &'static str>,
    pub(super) reflectors: HashMap<TypeId, Box<dyn ReflectAccessor>>,
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
            storages: HashMap::new(),
            type_names: HashMap::new(),
            reflectors: HashMap::new(),
        }
    }

    /// Registers a CPU-only component type.
    ///
    /// Does nothing if the type is already registered.
    pub fn register_cpu<T: Component>(&mut self) {
        let type_id = TypeId::of::<T>();
        self.storages
            .entry(type_id)
            .or_insert_with(|| UnsafeCell::new(Box::new(ComponentStorage::<T>::new())));
        self.type_names
            .entry(type_id)
            .or_insert_with(|| std::any::type_name::<T>());
    }

    /// Returns an immutable reference to a CPU component storage.
    pub fn get_cpu<T: Component>(&self) -> Option<&ComponentStorage<T>> {
        self.storages.get(&TypeId::of::<T>()).and_then(|cell| {
            // SAFETY: No mutable references exist (we have &self, not via query).
            let storage = unsafe { &*cell.get() };
            storage.as_any().downcast_ref()
        })
    }

    /// Returns a mutable reference to a CPU component storage.
    pub fn get_cpu_mut<T: Component>(&mut self) -> Option<&mut ComponentStorage<T>> {
        self.storages.get_mut(&TypeId::of::<T>()).and_then(|cell| {
            // SAFETY: We have &mut self, so exclusive access is guaranteed.
            let storage = cell.get_mut();
            storage.as_any_mut().downcast_mut()
        })
    }

    /// Removes `entity` from all registered storages.
    pub fn remove_entity(&mut self, entity: Entity) {
        for cell in self.storages.values_mut() {
            cell.get_mut().remove_entity(entity);
        }
    }

    /// Removes a single component from `entity` by `TypeId`.
    ///
    /// Does nothing if the type is not registered or the entity doesn't have it.
    pub fn remove_component(&mut self, entity: Entity, type_id: &TypeId) {
        if let Some(cell) = self.storages.get_mut(type_id) {
            cell.get_mut().remove_entity(entity);
        }
    }

    /// Returns `true` if a storage is registered for the given `TypeId`.
    pub fn contains_type(&self, type_id: &TypeId) -> bool {
        self.storages.contains_key(type_id)
    }

    /// Returns the human-readable type name for a registered component.
    pub fn component_name(&self, type_id: &TypeId) -> Option<&'static str> {
        self.type_names.get(type_id).copied()
    }

    // -- Reflected registration -------------------------------------------------

    /// Registers a CPU-only component with reflection support.
    ///
    /// The component must implement both [`Component`] and [`Reflect`].
    /// Does nothing if the type is already registered.
    pub fn register_cpu_reflected<T: Component + Reflect>(&mut self) {
        self.register_cpu::<T>();
        self.reflectors
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(TypedReflectAccessor::<T>::new_cpu()));
    }

    // -- Reflection API -------------------------------------------------------

    /// Returns `true` if a reflection accessor is registered for `type_id`.
    pub fn has_reflector(&self, type_id: &TypeId) -> bool {
        self.reflectors.contains_key(type_id)
    }

    /// Returns field metadata for a reflected component type.
    pub fn reflect_field_metas(&self, type_id: &TypeId) -> Option<&'static [FieldMeta]> {
        self.reflectors.get(type_id).map(|r| r.fields())
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
        let accessor = self.reflectors.get(type_id)?;
        let storage = self.storages.get(type_id)?;
        // SAFETY: We have &self and no mutable references are active.
        let storage_ref = unsafe { &**storage.get() };
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
        let accessor = self
            .reflectors
            .get(type_id)
            .ok_or(ReflectError::ComponentNotFound)?;
        let storage = self
            .storages
            .get(type_id)
            .ok_or(ReflectError::ComponentNotFound)?;
        // SAFETY: We have &mut self, so exclusive access is guaranteed.
        let storage_mut = unsafe { &mut **storage.get() };
        accessor.set_field(storage_mut, entity, field, value)
    }

    /// Returns the inspector visibility for a reflected component type.
    /// Returns the inspector visibility for a reflected component type.
    pub fn reflect_inspector_visibility(&self, type_id: &TypeId) -> Option<InspectorVisibility> {
        self.reflectors
            .get(type_id)
            .map(|r| r.inspector_visibility())
    }

    /// Returns the editor category for a reflected component type, if any.
    pub fn reflect_category(&self, type_id: &TypeId) -> Option<&'static str> {
        self.reflectors.get(type_id).and_then(|r| r.category())
    }

    /// Returns all `TypeId`s that have a registered reflector.
    pub fn reflected_type_ids(&self) -> Vec<TypeId> {
        self.reflectors.keys().copied().collect()
    }

    /// Looks up a `TypeId` by its full type name string.
    ///
    /// Linear scan of the `type_names` map. Only called at scene load time.
    pub fn type_id_by_name(&self, name: &str) -> Option<TypeId> {
        self.type_names
            .iter()
            .find(|(_, n)| **n == name)
            .map(|(tid, _)| *tid)
    }

    /// Returns all registered component types with their human-readable names.
    pub fn all_type_names(&self) -> Vec<(TypeId, &'static str)> {
        self.type_names
            .iter()
            .map(|(tid, name)| (*tid, *name))
            .collect()
    }

    /// Returns all reflected types with their human-readable names.
    pub fn reflected_type_names(&self) -> Vec<(TypeId, &'static str)> {
        self.reflectors
            .keys()
            .filter_map(|tid| self.type_names.get(tid).map(|name| (*tid, *name)))
            .collect()
    }

    /// Inserts a default reflected component for an entity.
    ///
    /// Returns `true` if the component was inserted successfully.
    /// Returns `false` if the type has no reflector, no storage, or insert failed.
    pub fn insert_default_reflected(&mut self, type_id: &TypeId, entity: Entity) -> bool {
        let Some(accessor) = self.reflectors.get(type_id) else {
            return false;
        };
        let Some(cell) = self.storages.get(type_id) else {
            return false;
        };
        // SAFETY: We have &mut self, exclusive access guaranteed.
        let storage = unsafe { &mut **cell.get() };
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
        self.storages
            .get(type_id)
            .map(|cell| unsafe { &**cell.get() })
    }

    /// Returns a mutable reference to the type-erased storage for the given `TypeId`.
    ///
    /// # Safety
    ///
    /// Caller must ensure no other reference (mutable or immutable) to the
    /// same `TypeId` storage is active.
    pub(crate) unsafe fn storage_mut(&self, type_id: &TypeId) -> Option<&mut dyn AnyStorage> {
        self.storages
            .get(type_id)
            .map(|cell| unsafe { &mut **cell.get() })
    }
}

impl Default for ComponentRegistry {
    fn default() -> Self {
        Self::new()
    }
}
