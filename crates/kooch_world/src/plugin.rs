//! [`WorldStreamingPlugin`]: registers [`ChunkManager`] and [`LodRingConfig`] and schedules
//! activation. It attaches no [`StreamingFocus`]; the game or editor chooses which entities drive
//! streaming.

use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentRegistry;

use crate::activation::activation_system;
use crate::focus::StreamingFocus;
use crate::focus_cache::FocusCacheState;
use crate::lod::LodRingConfig;
use crate::manager::ChunkManager;

/// Chunks loaded per activation tick; the synchronous loader costs nothing, but the bound is the
/// pattern async loading keeps.
pub const DEFAULT_MAX_LOADS_PER_FRAME: usize = 8;

/// Per-frame unload budget. Eviction listeners (e.g. #309 Edit Baker
/// flushes) can be expensive, so this caps the wall-clock cost of a
/// single frame's unloads.
pub const DEFAULT_MAX_UNLOADS_PER_FRAME: usize = 4;

/// Plugin that registers world-streaming machinery on an `App`.
pub struct WorldStreamingPlugin;

impl Plugin for WorldStreamingPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ChunkManager::default());
        app.insert_resource(LodRingConfig::default());
        app.insert_resource(FocusCacheState::default());

        // `register_cpu_reflected`, or the editor's inspector and spawn flows cannot build a
        // default and `insert_default_reflected` silently does nothing.
        if let Some(registry) = app.resources_mut().get_mut::<ComponentRegistry>() {
            registry.register_cpu_reflected::<StreamingFocus>();
        }

        // Cache-gated by `FocusCacheState`: with no boundary crossed the system returns without
        // enumerating the grid — 41 M pending entries in 30 s before (#315).
        app.add_system(kooch_core::stage::Stage::PreUpdate, world_streaming_system);
    }

    fn name(&self) -> &str {
        "WorldStreamingPlugin"
    }
}

/// Lifts `ChunkManager` and `FocusCacheState` out of `Resources`, because activation reads
/// `&Resources` for the focus query while mutating both.
pub fn world_streaming_system(resources: &mut Resources) {
    let Some(mut manager) = resources.remove::<ChunkManager>() else {
        return;
    };
    let Some(mut cache) = resources.remove::<FocusCacheState>() else {
        // Put the manager back if we bail.
        resources.insert(manager);
        return;
    };
    let config = resources
        .get::<LodRingConfig>()
        .cloned()
        .unwrap_or_default();

    activation_system(resources, &mut cache, &mut manager, &config);
    let (loaded, unloaded) =
        manager.process_queues(DEFAULT_MAX_LOADS_PER_FRAME, DEFAULT_MAX_UNLOADS_PER_FRAME);

    if loaded > 0 || unloaded > 0 {
        tracing::trace!(
            target: "kooch_world",
            loaded,
            unloaded,
            active = manager.loaded_count(),
            pending_load = manager.pending_load_count(),
            pending_unload = manager.pending_unload_count(),
            "chunk streaming tick"
        );
    }

    resources.insert(manager);
    resources.insert(cache);
}

#[cfg(test)]
mod tests;
