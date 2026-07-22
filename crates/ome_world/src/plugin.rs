//! [`WorldStreamingPlugin`] — wires the chunk streaming subsystem into
//! an `App`. Registers the [`ChunkManager`] + [`LodRingConfig`]
//! resources and adds the activation + processing system to the
//! schedule.
//!
//! Consumers (editor / game) are expected to attach
//! [`StreamingFocus`] to whichever entities should drive streaming —
//! typically the active camera, plus any AI / event entity that
//! gameplay declares as a focus. The plugin does NOT auto-attach to a
//! camera so it stays composable with custom focus strategies.

use ome_core::app::App;
use ome_core::plugin::Plugin;
use ome_core::resource::Resources;
use ome_ecs::component::ComponentRegistry;

use crate::activation::activation_system;
use crate::focus::StreamingFocus;
use crate::focus_cache::FocusCacheState;
use crate::lod::LodRingConfig;
use crate::manager::ChunkManager;

/// Per-frame load budget. Caps how many chunks transition Unloaded →
/// Loaded in a single activation tick. Picked low for the warmup —
/// the synchronous loader has zero cost, but bounded budget is the
/// pattern future async loading will keep.
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

        // Register the StreamingFocus component on the ECS registry.
        // Use `register_cpu_reflected` (NOT plain `register_cpu`) so the
        // editor's drag-and-drop / inspector / spawn flows can construct
        // a default instance via the Reflect accessor — without the
        // reflector, `insert_default_reflected` silently no-ops.
        if let Some(registry) = app.resources_mut().get_mut::<ComponentRegistry>() {
            registry.register_cpu_reflected::<StreamingFocus>();
        }

        // Re-enabled in #115 PR-2: the activation system is now cache-
        // gated by `FocusCacheState`. When no `StreamingFocus` has
        // crossed a chunk boundary on any LOD since the last tick, the
        // system early-returns without enumerating the grid — kills
        // the regression caught in PR #315 (41 M pending entries in
        // 30 s of editor runtime). Activation is distance-based; the
        // LBVH that was going to accelerate it went with the raymarcher
        // it was built for, and physics broadphase is rapier's job now.
        app.add_system(ome_core::stage::Stage::PreUpdate, world_streaming_system);
    }

    fn name(&self) -> &str {
        "WorldStreamingPlugin"
    }
}

/// Per-frame system: pull `ChunkManager` + `FocusCacheState` out,
/// read `LodRingConfig`, run the cached activation pass + drain the
/// queues with the per-frame budget, put the resources back. The
/// remove/insert dance is needed because `activation_system` reads
/// `&Resources` for the focus query while we mutate the manager and
/// the cache.
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
            target: "ome_world",
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
mod tests {
    use super::*;
    use ome_core::plugin::CorePlugin;
    use ome_ecs::plugin::EcsPlugin;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugin(CorePlugin);
        app.add_plugin(EcsPlugin);
        app.add_plugin(WorldStreamingPlugin);
        app
    }

    #[test]
    fn plugin_inserts_resources() {
        let app = test_app();
        assert!(app.resources().get::<ChunkManager>().is_some());
        assert!(app.resources().get::<LodRingConfig>().is_some());
    }

    #[test]
    fn system_runs_without_focuses() {
        let mut app = test_app();
        // No focus entities — system runs without panic and produces
        // no chunk activity.
        world_streaming_system(app.resources_mut());
        let m = app.resources().get::<ChunkManager>().unwrap();
        assert_eq!(m.loaded_count(), 0);
    }
}
