//! ECS plugin that registers the entity allocator and GPU sync system.

use ome_core::app::App;
use ome_core::plugin::Plugin;
use ome_core::stage::Stage;

use crate::allocator::EntityAllocator;
#[cfg(feature = "dynamic")]
use crate::entity::Entity;
use crate::gpu_sync::entity_gpu_sync_system;

/// Plugin that bootstraps the entity system.
///
/// Inserts a default [`EntityAllocator`] and registers
/// [`entity_gpu_sync_system`] in [`Stage::GpuSync`].
pub struct EcsPlugin;

impl Plugin for EcsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(EntityAllocator::new());
        app.add_system(Stage::GpuSync, entity_gpu_sync_system);

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
    fn plugin_registers_allocator() {
        let mut app = App::new();
        app.add_plugin(EcsPlugin);

        assert!(app.resources().get::<EntityAllocator>().is_some());
    }
}
