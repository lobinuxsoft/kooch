//! Component storage for the ECS.
//!
//! Provides dense GPU-backed storage ([`GpuComponentStorage`]) and CPU-only
//! storage ([`ComponentStorage`]) managed through a central
//! [`ComponentRegistry`].

pub mod cpu_storage;
pub mod gpu_storage;
pub mod names;
pub mod registry;
pub(crate) mod traits;

pub use cpu_storage::ComponentStorage;
pub use gpu_storage::GpuComponentStorage;
pub use names::{ComponentId, ComponentNames};
pub use registry::ComponentRegistry;
pub use traits::{Component, GpuComponent};

use ome_core::gpu::GpuContext;
use ome_core::resource::Resources;

use crate::allocator::EntityAllocator;

/// Removes despawned entities from all component storages.
///
/// Runs in [`Stage::GpuSync`](ome_core::stage::Stage::GpuSync) **before**
/// the entity and component GPU sync systems.
pub fn component_despawn_cleanup_system(resources: &mut Resources) {
    let despawned = resources
        .get_mut::<EntityAllocator>()
        .map(|a| a.take_pending_despawn())
        .unwrap_or_default();

    if despawned.is_empty() {
        return;
    }

    if let Some(mut registry) = resources.remove::<ComponentRegistry>() {
        for entity in &despawned {
            registry.remove_entity(*entity);
        }
        resources.insert(registry);
    }
}

/// Syncs all GPU-backed component storages to the GPU.
///
/// Runs in [`Stage::GpuSync`](ome_core::stage::Stage::GpuSync) **after**
/// the entity GPU sync system.
pub fn component_gpu_sync_system(resources: &mut Resources) {
    let capacity = resources
        .get::<EntityAllocator>()
        .map(|a| a.total_slots())
        .unwrap_or(0);

    let mut registry = resources.remove::<ComponentRegistry>();

    if let (Some(reg), Some(gpu)) = (registry.as_mut(), resources.get::<GpuContext>()) {
        reg.sync_all_gpu(gpu.device(), gpu.queue(), capacity);
    }

    if let Some(reg) = registry {
        resources.insert(reg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
    #[repr(C)]
    struct Velocity {
        vx: f32,
        vy: f32,
    }
    impl GpuComponent for Velocity {}

    struct Tag(String);
    impl Component for Tag {}

    #[test]
    fn despawn_cleanup_removes_components() {
        let mut resources = Resources::new();
        let mut alloc = EntityAllocator::with_capacity(4);
        let e = alloc.spawn();

        let mut registry = ComponentRegistry::new();
        registry.register_gpu::<Velocity>("velocity");
        registry.register_cpu::<Tag>();

        registry
            .get_gpu_mut::<Velocity>()
            .unwrap()
            .insert(e, Velocity { vx: 1.0, vy: 2.0 });
        registry
            .get_cpu_mut::<Tag>()
            .unwrap()
            .insert(e, Tag("test".into()));

        alloc.despawn(e);

        resources.insert(alloc);
        resources.insert(registry);

        component_despawn_cleanup_system(&mut resources);

        let registry = resources.get::<ComponentRegistry>().unwrap();
        assert!(!registry.get_gpu::<Velocity>().unwrap().contains(e));
        assert!(!registry.get_cpu::<Tag>().unwrap().contains(e));
    }

    #[test]
    fn despawn_cleanup_no_panic_without_registry() {
        let mut resources = Resources::new();
        let mut alloc = EntityAllocator::with_capacity(4);
        let e = alloc.spawn();
        alloc.despawn(e);
        resources.insert(alloc);

        // No registry — should not panic.
        component_despawn_cleanup_system(&mut resources);
    }

    #[test]
    fn despawn_cleanup_no_panic_without_allocator() {
        let mut resources = Resources::new();
        // No allocator — should not panic.
        component_despawn_cleanup_system(&mut resources);
    }

    #[test]
    fn gpu_sync_no_panic_without_gpu() {
        let mut resources = Resources::new();
        resources.insert(EntityAllocator::with_capacity(4));
        resources.insert(ComponentRegistry::new());

        // No GpuContext — headless mode, should not panic.
        component_gpu_sync_system(&mut resources);
    }

    #[test]
    fn gpu_sync_no_panic_without_registry() {
        let mut resources = Resources::new();
        resources.insert(EntityAllocator::with_capacity(4));

        // No registry — should not panic.
        component_gpu_sync_system(&mut resources);
    }

    #[test]
    fn gpu_sync_no_panic_without_allocator() {
        let mut resources = Resources::new();
        // No allocator — should not panic.
        component_gpu_sync_system(&mut resources);
    }
}
