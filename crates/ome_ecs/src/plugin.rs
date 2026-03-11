//! ECS plugin that registers the entity allocator and GPU sync system.

use ome_core::app::App;
use ome_core::plugin::Plugin;
use ome_core::stage::Stage;

use crate::allocator::EntityAllocator;
use crate::component::registry::ComponentRegistry;
use crate::component::{component_despawn_cleanup_system, component_gpu_sync_system};
#[cfg(feature = "dynamic")]
use crate::entity::Entity;
use crate::gpu_sync::entity_gpu_sync_system;
use crate::query::AccessTracker;

/// Plugin that bootstraps the entity and component systems.
///
/// Inserts [`EntityAllocator`] and [`ComponentRegistry`], then registers
/// the GPU sync systems in [`Stage::GpuSync`] in the correct order:
///
/// 1. `component_despawn_cleanup_system` — remove despawned entities from storages
/// 2. `entity_gpu_sync_system` — sync alive mask to GPU
/// 3. `component_gpu_sync_system` — upload dirty component data to GPU
pub struct EcsPlugin;

impl Plugin for EcsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(EntityAllocator::new());
        app.insert_resource(ComponentRegistry::new());
        app.insert_resource(AccessTracker::new());

        // Order within a stage is insertion order — these three MUST stay in this sequence.
        app.add_system(Stage::GpuSync, component_despawn_cleanup_system);
        app.add_system(Stage::GpuSync, entity_gpu_sync_system);
        app.add_system(Stage::GpuSync, component_gpu_sync_system);

        #[cfg(feature = "dynamic")]
        {
            use ome_core::dynamic::EntityBridge;
            use ome_plugin_api::types::{pack_entity, unpack_entity};

            app.insert_resource(EntityBridge::new(
                |resources| {
                    let alloc = resources
                        .get_mut::<EntityAllocator>()
                        .expect("EntityAllocator not found");
                    let entity = alloc.spawn();
                    pack_entity(entity.index(), entity.generation())
                },
                |resources, handle| {
                    let alloc = resources
                        .get_mut::<EntityAllocator>()
                        .expect("EntityAllocator not found");
                    let (index, generation) = unpack_entity(handle);
                    alloc.despawn(Entity::new(index, generation))
                },
            ));
        }
    }

    fn name(&self) -> &str {
        "EcsPlugin"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_registers_allocator_and_registry() {
        let mut app = App::new();
        app.add_plugin(EcsPlugin);

        assert!(app.resources().get::<EntityAllocator>().is_some());
        assert!(app.resources().get::<ComponentRegistry>().is_some());
        assert!(app.resources().get::<AccessTracker>().is_some());
    }
}
