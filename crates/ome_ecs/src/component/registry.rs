//! Type-erased component registry.
//!
//! [`ComponentRegistry`] maps `TypeId` → `Box<dyn AnyStorage>`, providing
//! typed access via downcasting and batch operations (remove entity from
//! all storages, sync all GPU storages).

use std::any::TypeId;
use std::cell::UnsafeCell;
use std::collections::HashMap;

use wgpu::{Device, Queue};

use crate::entity::Entity;

use super::cpu_storage::ComponentStorage;
use super::gpu_storage::GpuComponentStorage;
use super::traits::{AnyStorage, Component, GpuComponent};

/// Central registry for all component storages.
///
/// Stores one [`GpuComponentStorage<T>`] or [`ComponentStorage<T>`] per
/// registered component type, keyed by `TypeId`.
///
/// Uses [`UnsafeCell`] internally to allow the query system to borrow
/// multiple storages simultaneously. Safety is enforced at the query
/// level through runtime access tracking.
pub struct ComponentRegistry {
    storages: HashMap<TypeId, UnsafeCell<Box<dyn AnyStorage>>>,
    type_names: HashMap<TypeId, &'static str>,
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
        }
    }

    /// Registers a GPU-backed component type.
    ///
    /// `label` is used as the GPU buffer debug label.
    /// Does nothing if the type is already registered.
    pub fn register_gpu<T: GpuComponent>(&mut self, label: &str) {
        let type_id = TypeId::of::<T>();
        self.storages
            .entry(type_id)
            .or_insert_with(|| UnsafeCell::new(Box::new(GpuComponentStorage::<T>::new(label))));
        self.type_names
            .entry(type_id)
            .or_insert_with(|| std::any::type_name::<T>());
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

    /// Returns an immutable reference to a GPU component storage.
    pub fn get_gpu<T: GpuComponent>(&self) -> Option<&GpuComponentStorage<T>> {
        self.storages.get(&TypeId::of::<T>()).and_then(|cell| {
            // SAFETY: No mutable references exist (we have &self, not via query).
            let storage = unsafe { &*cell.get() };
            storage.as_any().downcast_ref()
        })
    }

    /// Returns a mutable reference to a GPU component storage.
    pub fn get_gpu_mut<T: GpuComponent>(&mut self) -> Option<&mut GpuComponentStorage<T>> {
        self.storages.get_mut(&TypeId::of::<T>()).and_then(|cell| {
            // SAFETY: We have &mut self, so exclusive access is guaranteed.
            let storage = cell.get_mut();
            storage.as_any_mut().downcast_mut()
        })
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

    /// Syncs all GPU-backed storages to the GPU.
    ///
    /// CPU-only storages have a no-op `sync_gpu` so this is safe to call
    /// on the entire registry.
    pub fn sync_all_gpu(&mut self, device: &Device, queue: &Queue, capacity: u32) {
        for cell in self.storages.values_mut() {
            cell.get_mut().sync_gpu(device, queue, capacity);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
    #[repr(C)]
    struct Position {
        x: f32,
        y: f32,
    }
    impl GpuComponent for Position {}

    struct Name(String);
    impl Component for Name {}

    fn entity(index: u32) -> Entity {
        Entity::new(index, 0)
    }

    #[test]
    fn register_and_get_gpu() {
        let mut registry = ComponentRegistry::new();
        registry.register_gpu::<Position>("position");

        let storage = registry.get_gpu::<Position>().unwrap();
        assert!(storage.is_empty());
    }

    #[test]
    fn register_and_get_cpu() {
        let mut registry = ComponentRegistry::new();
        registry.register_cpu::<Name>();

        let storage = registry.get_cpu::<Name>().unwrap();
        assert!(storage.is_empty());
    }

    #[test]
    fn get_unregistered_returns_none() {
        let registry = ComponentRegistry::new();
        assert!(registry.get_gpu::<Position>().is_none());
        assert!(registry.get_cpu::<Name>().is_none());
    }

    #[test]
    fn register_idempotent() {
        let mut registry = ComponentRegistry::new();
        registry.register_gpu::<Position>("position");

        // Insert data via the storage.
        registry
            .get_gpu_mut::<Position>()
            .unwrap()
            .insert(entity(0), Position { x: 1.0, y: 2.0 });

        // Re-registering should NOT reset the storage.
        registry.register_gpu::<Position>("position");
        assert_eq!(registry.get_gpu::<Position>().unwrap().len(), 1);
    }

    #[test]
    fn insert_and_retrieve_gpu_components() {
        let mut registry = ComponentRegistry::new();
        registry.register_gpu::<Position>("position");

        let e = entity(5);
        registry
            .get_gpu_mut::<Position>()
            .unwrap()
            .insert(e, Position { x: 3.0, y: 4.0 });

        let pos = registry.get_gpu::<Position>().unwrap().get(e).unwrap();
        assert_eq!(pos.x, 3.0);
        assert_eq!(pos.y, 4.0);
    }

    #[test]
    fn insert_and_retrieve_cpu_components() {
        let mut registry = ComponentRegistry::new();
        registry.register_cpu::<Name>();

        let e = entity(0);
        registry
            .get_cpu_mut::<Name>()
            .unwrap()
            .insert(e, Name("Alice".into()));

        let name = registry.get_cpu::<Name>().unwrap().get(e).unwrap();
        assert_eq!(name.0, "Alice");
    }

    #[test]
    fn remove_entity_from_all_storages() {
        let mut registry = ComponentRegistry::new();
        registry.register_gpu::<Position>("position");
        registry.register_cpu::<Name>();

        let e = entity(0);
        registry
            .get_gpu_mut::<Position>()
            .unwrap()
            .insert(e, Position { x: 1.0, y: 2.0 });
        registry
            .get_cpu_mut::<Name>()
            .unwrap()
            .insert(e, Name("Bob".into()));

        registry.remove_entity(e);

        assert!(!registry.get_gpu::<Position>().unwrap().contains(e));
        assert!(!registry.get_cpu::<Name>().unwrap().contains(e));
    }

    #[test]
    fn mixed_gpu_and_cpu_storages() {
        let mut registry = ComponentRegistry::new();
        registry.register_gpu::<Position>("position");
        registry.register_cpu::<Name>();

        let e1 = entity(0);
        let e2 = entity(1);

        registry
            .get_gpu_mut::<Position>()
            .unwrap()
            .insert(e1, Position { x: 1.0, y: 0.0 });
        registry
            .get_gpu_mut::<Position>()
            .unwrap()
            .insert(e2, Position { x: 2.0, y: 0.0 });
        registry
            .get_cpu_mut::<Name>()
            .unwrap()
            .insert(e1, Name("A".into()));

        assert_eq!(registry.get_gpu::<Position>().unwrap().len(), 2);
        assert_eq!(registry.get_cpu::<Name>().unwrap().len(), 1);
    }

    #[test]
    fn contains_type_check() {
        let mut registry = ComponentRegistry::new();
        assert!(!registry.contains_type(&TypeId::of::<Position>()));

        registry.register_gpu::<Position>("position");
        assert!(registry.contains_type(&TypeId::of::<Position>()));
    }
}
