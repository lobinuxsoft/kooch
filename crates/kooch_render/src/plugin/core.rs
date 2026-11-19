//! What every host that draws through the meshlet stage needs, whoever drives its frames: the
//! game's frame below, or the editor's views (#1928).

use kooch_core::app::App;
use kooch_core::gpu::GpuContext;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::stage::Stage;

use crate::VIEWPORT_DEPTH_FORMAT;
use crate::meshlet::{MeshletBlit, MeshletDebugCaps, MeshletRenderStage, MeshletRenderStageConfig};
use crate::sky::SkyRenderPass;
use crate::vbuf64::Vbuf64Support;

/// The meshlet stage, its sky and blit, and the stores other crates feed it through. Drives no
/// frame: [`super::RenderPlugin`] adds the game's, an editor adds its own views.
#[derive(Default)]
pub struct RenderCorePlugin;

impl Plugin for RenderCorePlugin {
    fn build(&self, app: &mut App) {
        // 🔴 Inserted here, not by whoever generates a mesh: the store belongs to this crate and the
        // drain is in this crate's meshlet sync.
        app.insert_resource(crate::meshlet::GeneratedMeshes::new());
        app.insert_resource(crate::meshlet::GpuMeshes::new());
        app.add_system(Stage::Startup, init_renderers);
    }

    fn name(&self) -> &str {
        "RenderCorePlugin"
    }
}

/// Builds the stage and what draws beside it, once a GPU is up. Idempotent: a host that needs the
/// stage before its own startup runs calls it first.
pub fn init_renderers(resources: &mut Resources) {
    if resources.get::<MeshletRenderStage>().is_some() {
        return;
    }
    let Some(gpu) = resources.get::<GpuContext>() else {
        // The context is built after Startup in a game; the frame retries.
        tracing::debug!("RenderCorePlugin: GpuContext not up yet, deferring init to the retry");
        return;
    };
    let vbuf64 = Vbuf64Support::detect(gpu.device());
    let debug_caps = MeshletDebugCaps::detect(gpu.device());
    let sky_pass = SkyRenderPass::new(gpu.device(), gpu.format());
    let mut meshlet_stage = MeshletRenderStage::new(
        gpu.device(),
        MeshletRenderStageConfig {
            size: gpu.size(),
            vbuf64,
            debug_caps,
            ..Default::default()
        },
    );
    // A no-op on adapters without `TIMESTAMP_QUERY`.
    meshlet_stage.enable_gpu_timers(gpu.device(), gpu.queue(), gpu.adapter());
    let meshlet_blit = MeshletBlit::new(gpu.device(), gpu.format(), VIEWPORT_DEPTH_FORMAT);
    // #785 — per-pass GPU timings. `None` in a build without the `gpu-profiler` feature.
    let gpu_scopes = kooch_core::gpu::GpuScopes::new(gpu.device(), gpu.queue());
    resources.insert(vbuf64);
    resources.insert(debug_caps);
    resources.insert(sky_pass);
    resources.insert(meshlet_stage);
    resources.insert(meshlet_blit);
    if let Some(gpu_scopes) = gpu_scopes {
        resources.insert(gpu_scopes);
        tracing::info!("RenderCorePlugin: GPU scopes enabled");
    }
    tracing::info!("RenderCorePlugin: renderers initialized (sky + meshlet)");
}
