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

        // Register the StreamingFocus component on the ECS registry.
        // Use `register_cpu_reflected` (NOT plain `register_cpu`) so the
        // editor's drag-and-drop / inspector / spawn flows can construct
        // a default instance via the Reflect accessor — without the
        // reflector, `insert_default_reflected` silently no-ops.
        if let Some(registry) = app.resources_mut().get_mut::<ComponentRegistry>() {
            registry.register_cpu_reflected::<StreamingFocus>();
        }

        // NOTE: the per-frame activation system is intentionally NOT
        // registered here. The current `activation_system` does brute-
        // force grid iteration over the LOD ring radii, which produces
        // millions of pending chunk requests per frame at gameplay-
        // realistic radii (e.g. 32 km LOD-3 ring with 512 m chunks =
        // ~238 k chunks per ring per frame). Editor tested at 41 M
        // entries pending after 30 s of runtime.
        //
        // The proper fix is structural: a BVH / octree (issue #115) so
        // the activation queries each LOD ring in O(log N) instead of
        // O(N³). When #115 lands, the schedule registration goes back
        // here and the brute-force `chunks_within_sphere` is replaced
        // by an octree query inside `activation_system`.
        //
        // Until then, `activation_system` and `activate_chunks` remain
        // public helpers (callable from tests / manual tools) — the
        // math (AABB-vs-sphere filter, distance² priority) is correct
        // and survives the refactor.
    }

    fn name(&self) -> &str {
        "WorldStreamingPlugin"
    }
}

/// Per-frame system: pull `ChunkManager` out, read `LodRingConfig`,
/// run the activation pass + drain the queues with the per-frame
/// budget, put the manager back. The remove/insert dance is needed
/// because `activation_system` reads `&Resources` for the focus query
/// while we mutate the manager.
///
/// Currently unregistered from the schedule (see `WorldStreamingPlugin::build`
/// for the rationale — brute-force activation is O(N³), waiting on
/// #115 PR-2 to plug in BVH-backed queries). Exposed publicly so
/// integration tests / debug tools can drive a manual tick.
pub fn world_streaming_system(resources: &mut Resources) {
    let Some(mut manager) = resources.remove::<ChunkManager>() else {
        return;
    };
    let config = resources
        .get::<LodRingConfig>()
        .cloned()
        .unwrap_or_default();

    activation_system(resources, &mut manager, &config);
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
