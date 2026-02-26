//! Type-erased component registry.
//!
//! [`ComponentRegistry`] maps `TypeId` → `Box<dyn AnyStorage>`, providing
//! typed access via downcasting and batch operations (remove entity from
//! all storages, sync all GPU storages).

use std::any::TypeId;
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
pub struct ComponentRegistry {
    storages: HashMap<TypeId, Box<dyn AnyStorage>>,
}

impl ComponentRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            storages: HashMap::new(),
        }
    }

    /// Registers a GPU-backed component type.
    ///
    /// `label` is used as the GPU buffer debug label.
    /// Does nothing if the type is already registered.
    pub fn register_gpu<T: GpuComponent>(&mut self, label: &str) {
        self.storages
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(GpuComponentStorage::<T>::new(label)));
    }

    /// Registers a CPU-only component type.
    ///
    /// Does nothing if the type is already registered.
    pub fn register_cpu<T: Component>(&mut self) {
        self.storages
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(ComponentStorage::<T>::new()));
    }

    /// Returns an immutable reference to a GPU component storage.
    pub fn get_gpu<T: GpuComponent>(&self) -> Option<&GpuComponentStorage<T>> {
        self.storages
            .get(&TypeId::of::<T>())
            .and_then(|s| s.as_any().downcast_ref())
    }

    /// Returns a mutable reference to a GPU component storage.
    pub fn get_gpu_mut<T: GpuComponent>(&mut self) -> Option<&mut GpuComponentStorage<T>> {
        self.storages
            .get_mut(&TypeId::of::<T>())
            .and_then(|s| s.as_any_mut().downcast_mut())
    }

    /// Returns an immutable reference to a CPU component storage.
    pub fn get_cpu<T: Component>(&self) -> Option<&ComponentStorage<T>> {
        self.storages
            .get(&TypeId::of::<T>())
            .and_then(|s| s.as_any().downcast_ref())
    }

    /// Returns a mutable reference to a CPU component storage.
    pub fn get_cpu_mut<T: Component>(&mut self) -> Option<&mut ComponentStorage<T>> {
        self.storages
            .get_mut(&TypeId::of::<T>())
            .and_then(|s| s.as_any_mut().downcast_mut())
    }

    /// Removes `entity` from all registered storages.
    pub fn remove_entity(&mut self, entity: Entity) {
        for storage in self.storages.values_mut() {
            storage.remove_entity(entity);
        }
    }

    /// Syncs all GPU-backed storages to the GPU.
    ///
    /// CPU-only storages have a no-op `sync_gpu` so this is safe to call
    /// on the entire registry.
    pub fn sync_all_gpu(&mut self, device: &Device, queue: &Queue, capacity: u32) {
        for storage in self.storages.values_mut() {
            storage.sync_gpu(device, queue, capacity);
        }
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
}
