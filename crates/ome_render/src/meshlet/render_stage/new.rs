use std::sync::Arc;

use super::super::deferred::{MeshletDeferredShader, DEFERRED_COLOR_FORMAT};
use super::super::dispatcher::MeshletCull;
use super::super::gpu_meshlet::meshlet_bind_group_layout;
use super::super::gpu_timers::MeshletGpuTimers;
use super::super::scene::MeshletScene;
use super::super::system::MeshletPipeline;
use super::super::vbuf64_stage::Vbuf64Stage;
use super::super::vis_buffer::{MeshletVisRasterizer, VISIBILITY_BUFFER_FORMAT};
use super::super::DEFAULT_MAX_TRIANGLES;
use super::config::MeshletRenderStageConfig;
use super::helpers::{create_2d_attachment, depth_sample_view, render_target_byte_estimate};
use super::stage::MeshletRenderStage;
use crate::hi_z::HiZ;
use crate::perf::EngineVramTracker;

impl MeshletRenderStage {
    pub fn new(device: &wgpu::Device, config: MeshletRenderStageConfig) -> Self {
        let MeshletRenderStageConfig {
            size,
            instance_capacity,
            meshlet_capacity,
            vbuf64,
            debug_caps,
        } = config;
        assert!(size.0 > 0 && size.1 > 0, "MeshletRenderStage size must be > 0");
        assert!(
            instance_capacity > 0,
            "MeshletRenderStage instance_capacity must be > 0"
        );

        let meshlet_bgl = meshlet_bind_group_layout(device);

        let cull = MeshletCull::new(device, meshlet_capacity, DEFAULT_MAX_TRIANGLES as u32);
        let scene = MeshletScene::new(device, instance_capacity);
        let rasterizer = MeshletVisRasterizer::new(
            device,
            Some(wgpu::TextureFormat::Depth32Float),
            cull.meshlet_bind_group_layout(),
            None,
        );
        let deferred = MeshletDeferredShader::new(device, cull.meshlet_bind_group_layout());

        let (vbuf_texture, vbuf_view) = create_2d_attachment(
            device,
            "meshlet_render_stage_vbuf",
            size,
            VISIBILITY_BUFFER_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let (depth_texture, depth_view) = create_2d_attachment(
            device,
            "meshlet_render_stage_depth",
            size,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let depth_sample_view = depth_sample_view(&depth_texture);
        let (color_texture, color_view) = create_2d_attachment(
            device,
            "meshlet_render_stage_color",
            size,
            DEFERRED_COLOR_FORMAT,
            wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );

        // Hi-Z pyramids stay None until the SPD-backed orchestrator
        // (#486) toggles them on. The single-pass orchestrator never
        // samples them — allocating at construction would just waste
        // VRAM and surface wgpu noise from the editor's per-frame
        // placeholder stage.
        let hiz_prev: Option<HiZ> = None;
        let hiz_curr: Option<HiZ> = None;

        // #493: opportunistic atomic R64 vbuf path. The legacy R32Uint
        // resources above stay live regardless — this is purely an
        // additional pipeline the orchestrator picks when the device
        // supports it.
        // #454: per-pixel R32Uint atomic accumulator that backs the
        // TriangleDensity / Overdraw heatmap modes and the reject
        // overlay raster pass. Allocated only when the device exposes
        // `Features::TEXTURE_ATOMIC` so a baseline-safe adapter pays
        // zero VRAM for a feature its driver could not run.
        let (triangle_density_texture, triangle_density_view) =
            if debug_caps.supports_texture_atomic() {
                let (tex, view) = create_2d_attachment(
                    device,
                    "meshlet_render_stage_triangle_density",
                    size,
                    wgpu::TextureFormat::R32Uint,
                    wgpu::TextureUsages::STORAGE_BINDING
                        | wgpu::TextureUsages::COPY_DST
                        | wgpu::TextureUsages::COPY_SRC,
                );
                (Some(tex), Some(view))
            } else {
                (None, None)
            };

        // Reject-reason overlay (#454.4). Same atomic gate as the
        // density texture above — both ride the
        // `MeshletDebugCaps::supports_texture_atomic` baseline split.
        // Pre-baseline adapters get `None`; the dropdown filter keeps
        // the user from selecting a mode that would dispatch into
        // empty space.
        let reject_overlay = if debug_caps.supports_texture_atomic() {
            Some(super::super::reject_overlay::MeshletRejectOverlay::new(device, &cull))
        } else {
            None
        };

        let vbuf64_stage = if vbuf64.is_supported() {
            Some(Vbuf64Stage::new(
                device,
                cull.meshlet_bind_group_layout(),
                wgpu::TextureFormat::Depth32Float,
                size,
                None,
            ))
        } else {
            None
        };

        Self {
            pipeline: MeshletPipeline::new(),
            scene,
            cull,
            rasterizer,
            deferred,
            gpu_pool: None,
            pool_dirty: false,
            meshlet_bgl,
            vbuf64_stage,
            vbuf_view,
            depth_view,
            depth_sample_view,
            color_view,
            vbuf_texture,
            depth_texture,
            color_texture,
            triangle_density_texture,
            triangle_density_view,
            reject_overlay,
            stage_counters: super::super::stage_counters::MeshletStageCounters::new(device),
            size,
            instance_capacity,
            // GPU timers default to disabled — tests don't pay for
            // them, and the editor / game runtime opts in via
            // [`Self::enable_gpu_timers`] at startup once the queue
            // and adapter are available.
            gpu_timers: MeshletGpuTimers::new_disabled_for_default(),
            vram_tracker: None,
            hiz_prev,
            hiz_curr,
            hi_z_initialized: false,
            frame_bind_groups: [Vec::new(), Vec::new(), Vec::new()],
            frame_bind_groups_index: 0,
            retired_pyramids: [Vec::new(), Vec::new(), Vec::new()],
        }
    }

    /// Swaps the current Hi-Z pyramid into the `prev` slot. The
    /// SPD-backed orchestrator follow-up (#486) will call this at
    /// end of frame so the next frame's pass A reads the pyramid
    /// this frame just built. No-op when pyramids haven't been
    /// allocated yet.
    #[allow(dead_code)]
    pub(super) fn swap_hi_z_pyramids(&mut self) {
        std::mem::swap(&mut self.hiz_prev, &mut self.hiz_curr);
    }

    /// Read-only access to the pyramid pass A samples this frame.
    /// `None` until the SPD orchestrator (#486) allocates them.
    pub fn hi_z_prev(&self) -> Option<&HiZ> {
        self.hiz_prev.as_ref()
    }

    /// Read-only access to the pyramid pass B samples (= the one
    /// rebuilt from this frame's depth between cull A and cull B).
    /// `None` until the SPD orchestrator (#486) allocates them.
    pub fn hi_z_curr(&self) -> Option<&HiZ> {
        self.hiz_curr.as_ref()
    }

    /// Wires a shared engine VRAM tracker (#463.5). Called once at
    /// startup from the editor / game runtime; subsequent buffer +
    /// texture creations / pool registrations the stage controls
    /// will bump the counter so the perf HUD can report a meaningful
    /// engine footprint. Idempotent — replacing the tracker with a
    /// different `Arc` is safe but discards the previous counter
    /// state for THIS stage's contribution (use sparingly).
    pub fn set_vram_tracker(&mut self, tracker: Arc<EngineVramTracker>) {
        // Account for the persistent attachments we already created
        // in `new()` — vbuf, depth, color. Hi-Z pyramids are lazy
        // (`None` until the SPD orchestrator #486 turns them on);
        // the pyramid allocation will bump the tracker on its own
        // when it runs.
        let attachment_bytes = render_target_byte_estimate(self.size);
        let pyramid_bytes = self
            .hiz_prev
            .as_ref()
            .map(|p| p.byte_size())
            .unwrap_or(0)
            + self.hiz_curr.as_ref().map(|p| p.byte_size()).unwrap_or(0);
        tracker.add(attachment_bytes + pyramid_bytes);
        self.vram_tracker = Some(tracker);
    }

    /// Activates the GPU frame timer (#463.4). Call this once at
    /// startup from the editor / game runtime, passing the engine's
    /// [`GpuContext`](ome_core::gpu::GpuContext) device + queue +
    /// adapter. Adapters without `Features::TIMESTAMP_QUERY` get a
    /// no-op instance — the call is always safe.
    pub fn enable_gpu_timers(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        adapter: &wgpu::Adapter,
    ) {
        self.gpu_timers = MeshletGpuTimers::new(device, queue, adapter);
    }

    /// Most recent GPU frame time in milliseconds, or `None` if the
    /// adapter does not expose `TIMESTAMP_QUERY` or the first
    /// readback hasn't completed yet.
    pub fn gpu_frame_ms(&self) -> Option<f32> {
        self.gpu_timers.last_frame_ms()
    }

    pub fn pipeline(&self) -> &MeshletPipeline {
        &self.pipeline
    }

    pub fn pipeline_mut(&mut self) -> &mut MeshletPipeline {
        &mut self.pipeline
    }

    /// Read-only access to the cull dispatcher. Mainly here so
    /// integration tests can read back `visible_count` /
    /// `culled_count` to verify the Hi-Z 2-pass cull behaviour
    /// (#445) frame-to-frame.
    pub fn cull(&self) -> &MeshletCull {
        &self.cull
    }

    pub fn color_view(&self) -> &wgpu::TextureView {
        &self.color_view
    }

    pub fn vbuf_view(&self) -> &wgpu::TextureView {
        &self.vbuf_view
    }

    /// Per-pixel triangle-density accumulator (#454). `Some` only when
    /// the construction-time `MeshletDebugCaps.supports_texture_atomic`
    /// was true. Read by the deferred shader to colourise the
    /// TriangleDensity / Overdraw heatmaps and by the reject-overlay
    /// raster pass for the rejection-mode views.
    pub fn triangle_density_view(&self) -> Option<&wgpu::TextureView> {
        self.triangle_density_view.as_ref()
    }

    /// Backing texture for [`Self::triangle_density_view`]. Exposed so
    /// the per-frame orchestrator can clear it (`COPY_DST` / compute
    /// reset) before the raster passes that accumulate into it.
    pub fn triangle_density_texture(&self) -> Option<&wgpu::Texture> {
        self.triangle_density_texture.as_ref()
    }

    /// Underlying color texture (Rgba8Unorm). Exposed so callers can
    /// copy it out for readback or composite it onto another target.
    pub fn color_texture(&self) -> &wgpu::Texture {
        &self.color_texture
    }

    pub fn vbuf_texture(&self) -> &wgpu::Texture {
        &self.vbuf_texture
    }

    pub fn depth_texture(&self) -> &wgpu::Texture {
        &self.depth_texture
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn instance_capacity(&self) -> u32 {
        self.instance_capacity
    }
}
