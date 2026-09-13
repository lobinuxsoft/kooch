//! Component storage for the ECS.

pub mod cpu_storage;
pub mod dynamic_types;
pub mod names;
#[cfg(feature = "dynamic")]
pub mod plugin_bridge;
pub mod registry;
pub mod storage_id;
pub(crate) mod traits;

pub use cpu_storage::ComponentStorage;
pub use dynamic_types::{DynamicField, DynamicType, DynamicTypeRegistry};
pub use names::{ComponentId, ComponentNames};
pub use registry::ComponentRegistry;
pub use storage_id::StorageId;
pub use traits::Component;

use kooch_core::gpu::GpuContext;
use kooch_core::resource::Resources;

use crate::allocator::EntityAllocator;

/// Removes despawned entities from all component storages.
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
