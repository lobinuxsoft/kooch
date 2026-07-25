//! Component storage for the ECS.
//!
//! Provides CPU storage ([`ComponentStorage`]) managed through a central
//! [`ComponentRegistry`].
//!
//! There is no GPU-backed storage: it existed, nothing ever used it, and it
//! was removed in #603. Data reaches the GPU through the meshlet pipeline's
//! own instance buffers, assembled from a CPU query — one route, not two.

pub mod cpu_storage;
pub mod names;
pub mod registry;
pub(crate) mod traits;

pub use cpu_storage::ComponentStorage;
pub use names::{ComponentId, ComponentNames};
pub use registry::ComponentRegistry;
pub use traits::Component;

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

#[cfg(test)]
mod tests {
    use super::*;

    struct Tag(String);
    impl Component for Tag {}

    #[test]
    fn despawn_cleanup_removes_components() {
        let mut resources = Resources::new();
        let mut alloc = EntityAllocator::with_capacity(4);
        let e = alloc.spawn();

        let mut registry = ComponentRegistry::new();
        registry.register_cpu::<Tag>();

        registry
            .get_cpu_mut::<Tag>()
            .unwrap()
            .insert(e, Tag("test".into()));

        alloc.despawn(e);

        resources.insert(alloc);
        resources.insert(registry);

        component_despawn_cleanup_system(&mut resources);

        let registry = resources.get::<ComponentRegistry>().unwrap();
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
}
