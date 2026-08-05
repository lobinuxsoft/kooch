use std::sync::Arc;

use super::super::DEFAULT_MAX_TRIANGLES;
use super::super::deferred::{DEFERRED_COLOR_FORMAT, MeshletDeferredShader};
use super::super::dispatcher::{MeshletCull, MeshletCullPipelines};
use super::super::gpu_meshlet::meshlet_bind_group_layout;
use super::super::gpu_timers::MeshletGpuTimers;
use super::super::scene::MeshletScene;
use super::super::system::MeshletPipeline;
use super::super::vbuf64_stage::Vbuf64Stage;
use super::super::vis_buffer::{MeshletVisRasterizer, VISIBILITY_BUFFER_FORMAT};
use super::config::MeshletRenderStageConfig;
use super::helpers::{create_2d_attachment, depth_sample_view, render_target_byte_estimate};
use super::stage::MeshletRenderStage;
use super::stage::ViewId;
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
        assert!(
            size.0 > 0 && size.1 > 0,
            "MeshletRenderStage size must be > 0"
        );
        assert!(
            instance_capacity > 0,
            "MeshletRenderStage instance_capacity must be > 0"
        );

        let meshlet_bgl = meshlet_bind_group_layout(device);

        // Pipelines are shared by every view; only the buffers below
        // are per view (#592).
        let cull_pipelines = MeshletCullPipelines::new(device);
        let scene = MeshletScene::new(device, instance_capacity);
        let rasterizer = MeshletVisRasterizer::new(
            device,
            Some(wgpu::TextureFormat::Depth32Float),
            cull_pipelines.meshlet_bind_group_layout(),
            None,
        );
        let deferred =
            MeshletDeferredShader::new(device, cull_pipelines.meshlet_bind_group_layout());

        // Everything per view lives together — see `view_targets` for
        // why the boundary is drawn here rather than at the fields.
        let mut views = slotmap::SlotMap::with_key();
        let primary = views.insert(super::view_targets::MeshletView::new(
            device,
            size,
            debug_caps,
            vbuf64,
            cull_pipelines.meshlet_bind_group_layout(),
            meshlet_capacity,
            DEFAULT_MAX_TRIANGLES as u32,
        ));

        // Reject-reason overlay (#454.4). Same atomic gate as the
        // density texture above — both ride the
        // `MeshletDebugCaps::supports_texture_atomic` baseline split.
        // Pre-baseline adapters get `None`; the dropdown filter keeps
        // the user from selecting a mode that would dispatch into
        // empty space.
        let reject_overlay = if debug_caps.supports_texture_atomic() {
            Some(super::super::reject_overlay::MeshletRejectOverlay::new(
                device,
                &cull_pipelines,
            ))
        } else {
            None
        };

        Self {
            pipeline: MeshletPipeline::new(),
            scene,
            cull_pipelines,
            rasterizer,
            deferred,
            lights: kooch_lighting::GpuLights::new(device),
            gpu_pool: None,
            pool_dirty: false,
            meshlet_bgl,
            views,
            primary,
            config,
            reject_overlay,
            stage_counters: super::super::stage_counters::MeshletStageCounters::new(device),
            instance_capacity,
            // GPU timers default to disabled — tests don't pay for
            // them, and the editor / game runtime opts in via
            // [`Self::enable_gpu_timers`] at startup once the queue
            // and adapter are available.
            gpu_timers: MeshletGpuTimers::new_disabled_for_default(),
            vram_tracker: None,
            frame_bind_groups: [Vec::new(), Vec::new(), Vec::new()],
            frame_bind_groups_index: 0,
        }
    }

    /// Swaps the current Hi-Z pyramid into the `prev` slot. The
    /// SPD-backed orchestrator follow-up (#486) will call this at
    /// end of frame so the next frame's pass A reads the pyramid
    /// this frame just built. No-op when pyramids haven't been
    /// allocated yet.
    #[allow(dead_code)]
    pub(super) fn swap_hi_z_pyramids(&mut self) {
        let view = &mut self.views[self.primary];
        std::mem::swap(&mut view.hiz_prev, &mut view.hiz_curr);
    }

    /// Read-only access to the pyramid pass A samples this frame.
    /// `None` until the SPD orchestrator (#486) allocates them.
    pub fn hi_z_prev(&self) -> Option<&HiZ> {
        self.views[self.primary].hiz_prev.as_ref()
    }

    /// Read-only access to the pyramid pass B samples (= the one
    /// rebuilt from this frame's depth between cull A and cull B).
    /// `None` until the SPD orchestrator (#486) allocates them.
    pub fn hi_z_curr(&self) -> Option<&HiZ> {
        self.views[self.primary].hiz_curr.as_ref()
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
        let attachment_bytes = render_target_byte_estimate(self.views[self.primary].size);
        let pyramid_bytes = self.views[self.primary]
            .hiz_prev
            .as_ref()
            .map(|p| p.byte_size())
            .unwrap_or(0)
            + self.views[self.primary]
                .hiz_curr
                .as_ref()
                .map(|p| p.byte_size())
                .unwrap_or(0);
        tracker.add(attachment_bytes + pyramid_bytes);
        self.vram_tracker = Some(tracker);
    }

    /// Activates the GPU frame timer (#463.4). Call this once at
    /// startup from the editor / game runtime, passing the engine's
    /// [`GpuContext`](kooch_core::gpu::GpuContext) device + queue +
    /// adapter. Adapters without `Features::TIMESTAMP_QUERY` get a
    /// no-op instance — the call is always safe.
    pub fn enable_gpu_timers(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        adapter: &wgpu::Adapter,
    ) {
        // 3 stages per frame so the per-pass HUD (#252) can split
        // the timer into cull / raster / post on the R64 path and
        // pass A / Hi-Z build / pass B on the 2-pass path. The
        // path-specific `render_path_*` functions write labels into
        // `MeshletRenderStats::stage_timings`.
        self.gpu_timers = MeshletGpuTimers::new_with_stages(device, queue, adapter, 3);
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
        &self.views[self.primary].cull
    }

    pub fn color_view(&self) -> &wgpu::TextureView {
        &self.views[self.primary].color_view
    }

    pub fn vbuf_view(&self) -> &wgpu::TextureView {
        &self.views[self.primary].vbuf_view
    }

    /// Per-pixel triangle-density accumulator (#454). `Some` only when
    /// the construction-time `MeshletDebugCaps.supports_texture_atomic`
    /// was true. Read by the deferred shader to colourise the
    /// TriangleDensity / Overdraw heatmaps and by the reject-overlay
    /// raster pass for the rejection-mode views.
    pub fn triangle_density_view(&self) -> Option<&wgpu::TextureView> {
        self.views[self.primary].triangle_density_view.as_ref()
    }

    /// Backing texture for [`Self::triangle_density_view`]. Exposed so
    /// the per-frame orchestrator can clear it (`COPY_DST` / compute
    /// reset) before the raster passes that accumulate into it.
    pub fn triangle_density_texture(&self) -> Option<&wgpu::Texture> {
        self.views[self.primary].triangle_density_texture.as_ref()
    }

    /// Underlying color texture (Rgba8Unorm). Exposed so callers can
    /// copy it out for readback or composite it onto another target.
    pub fn color_texture(&self) -> &wgpu::Texture {
        &self.views[self.primary].color_texture
    }

    pub fn vbuf_texture(&self) -> &wgpu::Texture {
        &self.views[self.primary].vbuf_texture
    }

    pub fn depth_texture(&self) -> &wgpu::Texture {
        &self.views[self.primary].depth_texture
    }

    pub fn size(&self) -> (u32, u32) {
        self.views[self.primary].size
    }

    pub fn instance_capacity(&self) -> u32 {
        self.instance_capacity
    }

    /// This stage's primary view — the one the single-view accessors
    /// read.
    pub fn primary_view(&self) -> ViewId {
        self.primary
    }

    /// Adds a view at `size` and returns its handle.
    ///
    /// Cheap relative to a second stage: the caller keeps sharing this
    /// stage's mesh pool, scene instances and cull pipelines, and pays
    /// only for the attachments and cull buffers the new camera needs.
    /// That is the whole reason the split exists — `measure_mesh_pool`
    /// puts the pool at 6.33 MiB for four assets, and duplicating it
    /// per camera would buy nothing.
    pub fn create_view(&mut self, device: &wgpu::Device, size: (u32, u32)) -> ViewId {
        self.views.insert(super::view_targets::MeshletView::new(
            device,
            size,
            self.config.debug_caps,
            self.config.vbuf64,
            self.cull_pipelines.meshlet_bind_group_layout(),
            self.config.meshlet_capacity,
            DEFAULT_MAX_TRIANGLES as u32,
        ))
    }

    /// Drops a view and its attachments.
    ///
    /// Refuses to drop the primary — a stage with no view cannot
    /// render, and every single-view accessor would have to start
    /// returning `Option`. Returns whether anything was removed, so a
    /// double close is a `false` rather than a panic.
    pub fn destroy_view(&mut self, id: ViewId) -> bool {
        if id == self.primary {
            tracing::warn!(
                target: "kooch_render::meshlet::render",
                "refusing to destroy the primary view",
            );
            return false;
        }
        self.views.remove(id).is_some()
    }

    /// Number of live views, primary included.
    pub fn view_count(&self) -> usize {
        self.views.len()
    }

    /// Whether `id` still addresses a live view. A generational key, so
    /// this stays false once the view is destroyed rather than
    /// silently resolving to whichever view took its slot.
    pub fn has_view(&self, id: ViewId) -> bool {
        self.views.contains_key(id)
    }

    /// Colour target of `id`, or `None` if the handle is stale.
    pub fn view_color_view(&self, id: ViewId) -> Option<&wgpu::TextureView> {
        self.views.get(id).map(|v| &v.color_view)
    }

    /// Size of `id`, or `None` if the handle is stale.
    pub fn view_size(&self, id: ViewId) -> Option<(u32, u32)> {
        self.views.get(id).map(|v| v.size)
    }

    /// Cull buffers of `id`, or `None` if the handle is stale. Each
    /// view owns a set — sharing them is what makes two overlapping
    /// viewports cull each other's geometry away.
    pub fn view_cull(&self, id: ViewId) -> Option<&MeshletCull> {
        self.views.get(id).map(|v| &v.cull)
    }
}
