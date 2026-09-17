//! Engine render plugins.

pub mod assets;

mod frame;
mod meshlets;
mod present;

pub use assets::AssetPlugin;

use glam::Vec4;
use kooch_core::app::App;
use kooch_core::gpu::{GpuContext, TargetDesc, TargetId, TargetPool};
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::schedule::Order;
use kooch_core::stage::Stage;

use crate::VIEWPORT_DEPTH_FORMAT;
use crate::meshlet::{MeshletBlit, MeshletDebugCaps, MeshletRenderStage, MeshletRenderStageConfig};
use crate::sky::SkyRenderPass;
use crate::vbuf64::Vbuf64Support;

/// Fallback clear color when no `SkyRenderer` entity is active. Matches the
/// `SkyRenderer` component's default bottom gradient so play and edit modes
/// look identical out of the box.
const SKY_FALLBACK: Vec4 = Vec4::new(0.1, 0.2, 0.4, 1.0);

/// Plugin that installs the full render pipeline.
#[derive(Default)]
pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        // 🔴 Inserted here, not by whoever generates a mesh: the store belongs to this crate and the
        // drain is in this crate's meshlet sync.
        app.insert_resource(crate::meshlet::GeneratedMeshes::new());
        app.add_system(Stage::Startup, init_renderers);
        // Before the frame, and in `Render` rather than `Update`: the resource it reads is written
        // by `apply_render_settings_system` in `Update`, and a stage boundary is the only ordering
        // between two plugins that does not depend on which one registered first.
        app.add_system(Stage::Render, apply_presentation_system);
        // 🔴 The frame in three systems, each naming the one before it. The scene is recorded and
        // submitted BEFORE the swapchain image is asked for, and that order is now a constraint
        // rather than three calls inside one function (#392).
        app.add_ordered(
            Stage::Render,
            Order::after("apply_presentation_system"),
            frame::prepare_frame_system,
        );
        app.add_ordered(
            Stage::Render,
            Order::after("prepare_frame_system"),
            meshlets::render_meshlets_system,
        );
        app.add_ordered(
            Stage::Render,
            Order::after("render_meshlets_system"),
            present::present_frame_system,
        );
    }

    fn name(&self) -> &str {
        "RenderPlugin"
    }
}

/// Puts [`Presentation`](crate::quality::Presentation) on the surface.
fn apply_presentation_system(resources: &mut Resources) {
    let Some(wanted) = resources.get::<crate::quality::Presentation>().copied() else {
        return;
    };
    let Some(gpu) = resources.get_mut::<GpuContext>() else {
        return;
    };
    gpu.set_vsync(wanted_vsync(
        wanted.vsync,
        kooch_core::gpu::vsync_override(),
    ));
}

/// The precedence rule, split out so it is testable without a GPU.
fn wanted_vsync(asset: bool, over: Option<bool>) -> bool {
    over.unwrap_or(asset)
}

/// The surface-sized depth target, held from the pool (#392).
pub(super) struct GameDepth {
    target: TargetId,
    size: (u32, u32),
}

impl GameDepth {
    pub(super) fn new(device: &wgpu::Device, pool: &mut TargetPool, size: (u32, u32)) -> Self {
        Self {
            target: pool.acquire(device, "game_depth_texture", depth_desc(size)),
            size,
        }
    }

    /// Swaps the target for one of the new size. The old one goes back to the pool, which is what
    /// keeps a run of resizes from allocating a depth texture per resize.
    pub(super) fn ensure(
        &mut self,
        device: &wgpu::Device,
        pool: &mut TargetPool,
        size: (u32, u32),
    ) {
        if size == self.size {
            return;
        }
        pool.release(self.target);
        self.target = pool.acquire(device, "game_depth_texture", depth_desc(size));
        self.size = size;
    }

    pub(super) fn view(&self, pool: &TargetPool) -> Option<wgpu::TextureView> {
        pool.view(self.target).cloned()
    }
}

fn depth_desc(size: (u32, u32)) -> TargetDesc {
    TargetDesc::attachment(size, VIEWPORT_DEPTH_FORMAT)
        .with_usage(wgpu::TextureUsages::RENDER_ATTACHMENT)
}

fn init_renderers(resources: &mut Resources) {
    if resources.get::<MeshletRenderStage>().is_some() {
        return;
    }
    let Some(gpu) = resources.get::<GpuContext>() else {
        // The same ordinary path as the material pipeline's: the
        // context is built after Startup and the retry picks this up.
        tracing::debug!("RenderPlugin: GpuContext not up yet, deferring init to the retry");
        return;
    };
    let pipeline_cache = gpu.pipeline_cache();
    let vbuf64 = Vbuf64Support::detect(gpu.device());
    let debug_caps = MeshletDebugCaps::detect(gpu.device());
    let sky_pass = SkyRenderPass::new(gpu.device(), gpu.format(), pipeline_cache);
    let mut pool = TargetPool::default();
    let depth = GameDepth::new(gpu.device(), &mut pool, gpu.size());
    let meshlet_stage = MeshletRenderStage::new(
        gpu.device(),
        MeshletRenderStageConfig {
            size: gpu.size(),
            vbuf64,
            debug_caps,
            ..Default::default()
        },
    );
    let mut meshlet_stage = meshlet_stage;
    // The editor did this at its own startup and a game never did, so the
    // GPU frame time existed on this adapter and was reported by nobody.
    // A no-op on adapters without `TIMESTAMP_QUERY`.
    meshlet_stage.enable_gpu_timers(gpu.device(), gpu.queue(), gpu.adapter());

    let meshlet_blit = MeshletBlit::new(gpu.device(), gpu.format(), VIEWPORT_DEPTH_FORMAT);
    // #785 — per-pass GPU timings. `None` in a build without the
    // `gpu-profiler` feature, and the render code below asks for the
    // resource the same way either way.
    let gpu_scopes = kooch_core::gpu::GpuScopes::new(gpu.device(), gpu.queue());
    resources.insert(vbuf64);
    resources.insert(debug_caps);
    resources.insert(sky_pass);
    resources.insert(pool);
    resources.insert(depth);
    resources.insert(meshlet_stage);
    resources.insert(meshlet_blit);
    if let Some(gpu_scopes) = gpu_scopes {
        resources.insert(gpu_scopes);
        tracing::info!("RenderPlugin: GPU scopes enabled");
    }
    tracing::info!("RenderPlugin: renderers initialized (sky + meshlet)");
}

#[cfg(test)]
mod tests;
