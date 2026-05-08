use std::any::TypeId;

use crate::component::registry::ComponentRegistry;
use crate::component::traits::{Component, GpuComponent};
use crate::entity::Entity;

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
