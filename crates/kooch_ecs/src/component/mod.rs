//! Component storage for the ECS.
//!
//! Provides CPU storage ([`ComponentStorage`]) managed through a central
//! [`ComponentRegistry`].
//!
//! There is no GPU-backed storage: it existed, nothing ever used it, and it
//! was removed in #603. Data reaches the GPU through the meshlet pipeline's
//! own instance buffers, assembled from a CPU query — one route, not two.

pub mod cpu_storage;
pub mod dynamic_types;
pub mod names;
#[cfg(feature = "dynamic")]
pub mod plugin_bridge;
pub mod registry;
pub(crate) mod traits;

pub use cpu_storage::ComponentStorage;
pub use dynamic_types::{DynamicField, DynamicType, DynamicTypeRegistry};
pub use names::{ComponentId, ComponentNames};
pub use registry::ComponentRegistry;
pub use traits::Component;

use kooch_core::gpu::GpuContext;
use kooch_core::resource::Resources;

use crate::allocator::EntityAllocator;

/// Removes despawned entities from all component storages.
///
/// Runs in [`Stage::GpuSync`](kooch_core::stage::Stage::GpuSync) **before**
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
mod tests;
