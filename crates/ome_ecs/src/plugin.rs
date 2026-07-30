//! ECS plugin that registers the entity allocator and GPU sync system.

use ome_core::app::App;
use ome_core::plugin::Plugin;
use ome_core::stage::Stage;

use crate::allocator::EntityAllocator;
use crate::archetype_registry::ArchetypeRegistry;
use crate::commands::{Commands, commands_apply_system};
use crate::component::ComponentNames;
use crate::component::component_despawn_cleanup_system;
use crate::component::registry::ComponentRegistry;
use crate::directional_light::DirectionalLight;
use crate::dynamic_components::DynamicComponents;
#[cfg(feature = "dynamic")]
use crate::entity::Entity;
use crate::ephemeral::EphemeralComponents;
use crate::hierarchy::{
    Children, GlobalTransform, Parent, hierarchy_sync_system, transform_propagation_system,
};
use crate::lod_force_level::LodForceLevel;
use crate::mesh_renderer::MeshRenderer;
use crate::name::Name;
use crate::orthographic_camera::OrthographicCamera;
use crate::persistent_id::{PersistentId, PersistentIdAllocator};
use crate::perspective_camera::PerspectiveCamera;
use crate::point_light::PointLight;
use crate::query::AccessTracker;
use crate::scene_manager::SceneManager;
use crate::sky_renderer::SkyRenderer;
use crate::spot_light::SpotLight;
use crate::transform::Transform;

/// Plugin that bootstraps the entity and component systems.
///
/// Inserts [`EntityAllocator`] and [`ComponentRegistry`], then registers
/// the GPU sync systems in [`Stage::GpuSync`] in the correct order:
///
/// 1. `component_despawn_cleanup_system` — remove despawned entities from storages
/// 2. `entity_gpu_sync_system` — sync alive mask to GPU
/// 3. `component_gpu_sync_system` — upload dirty component data to GPU
pub struct EcsPlugin;

/// Registers built-in engine components (Transform, Name).
fn register_builtin_components(resources: &mut ome_core::resource::Resources) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.register_cpu_reflected::<Transform>();
        registry.register_cpu_reflected::<Name>();
        registry.register_cpu_reflected::<Parent>();
        registry.register_cpu_reflected::<Children>();
        registry.register_cpu_reflected::<GlobalTransform>();
        registry.register_cpu_reflected::<PerspectiveCamera>();
        registry.register_cpu_reflected::<OrthographicCamera>();
        registry.register_cpu_reflected::<DirectionalLight>();
        registry.register_cpu_reflected::<PointLight>();
        registry.register_cpu_reflected::<SpotLight>();
        registry.register_cpu_reflected::<SkyRenderer>();
        registry.register_cpu_reflected::<MeshRenderer>();
        // Ordinary scene data despite being an editor concept: the link
        // between an instance and its prefab has to survive closing the
        // editor, so it is written to the scene file like anything else.
        registry.register_cpu_reflected::<crate::prefab_instance::PrefabInstance>();
        registry.register_cpu_reflected::<LodForceLevel>();
        registry.register_cpu_reflected::<PersistentId>();
    }
}

impl Plugin for EcsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(EntityAllocator::new());
        app.insert_resource(ComponentRegistry::new());
        app.insert_resource(ArchetypeRegistry::new());
        app.insert_resource(AccessTracker::new());
        app.insert_resource(Commands::new());
        app.insert_resource(EphemeralComponents::new());
        app.insert_resource(DynamicComponents::new());
        app.insert_resource(ComponentNames::new());
        app.insert_resource(PersistentIdAllocator::new());
        app.insert_resource(SceneManager::new());

        // Register built-in components before user startup systems.
        app.add_system(Stage::Startup, register_builtin_components);

        // Order within a stage is insertion order — these MUST stay in this
        // sequence.
        // 1. Apply deferred commands (spawn/despawn/insert/remove).
        // 2. Clean up despawned entities from component storages.
        // Hierarchy sync and transform propagation run before GPU sync.
        app.add_system(Stage::PostUpdate, hierarchy_sync_system);
        app.add_system(Stage::PostUpdate, transform_propagation_system);

        app.add_system(Stage::GpuSync, commands_apply_system);
        app.add_system(Stage::GpuSync, component_despawn_cleanup_system);

        #[cfg(feature = "dynamic")]
        {
            use ome_core::dynamic::{ComponentBridge, EntityBridge};
            use ome_plugin_api::types::{pack_entity, unpack_entity};

            use crate::component::DynamicTypeRegistry;
            use crate::component::plugin_bridge::register_schema;

            app.insert_resource(DynamicTypeRegistry::new());

            // A plugin's component types have no TypeId here, so they
            // are registered by name into the registry the editor reads
            // beside the reflected one.
            app.insert_resource(ComponentBridge::new(|resources, schema| {
                let Some(registry) = resources.get_mut::<DynamicTypeRegistry>() else {
                    return Err(ome_plugin_api::component::RegisterError::NoRegistry);
                };
                // Until a plugin can be asked which one it is, the type's
                // own path prefix identifies the source — enough to drop
                // its types together on unload.
                let source = schema
                    .type_name
                    .split("::")
                    .next()
                    .unwrap_or(&schema.type_name)
                    .to_owned();
                register_schema(registry, schema, &source)
            }));

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
