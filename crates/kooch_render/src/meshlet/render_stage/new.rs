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
            // Nothing has selected a technique yet, so a fresh view
            // renders at its panel's size.
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
            // Allocated on the first frame that finds a sun; see the
            // field's doc for why not here.
            shadows: None,
            shadow_texels: 0,
            point_shadows_over_budget: false,
            point_shadow_holders: Vec::new(),
            upscale_technique: crate::quality::UpscaleTechnique::None,
            render_scale: 100,
            instance_bounds: Vec::new(),
            point_cube_cache: Vec::new(),
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
            frames_recorded: 0,
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

    /// The shadow atlas this stage drew into, if it has one.
    ///
    /// For tests and for a future debug view (#743): the atlas answers
    /// "did the pass record this occluder" directly, where the shaded
    /// frame answers it through the whole sampling path.
    pub fn shadow_atlas_texture(&self) -> Option<&wgpu::Texture> {
        self.shadows.as_ref().map(|s| s.atlas_texture())
    }

    /// The point-light cube array, for the same reason as the atlas
    /// above: a test that reads the map answers "is the occluder in
    /// there" without going through the sampling path, the filter, the
    /// bias and a surface shader — four places a picture can lie.
    pub fn shadow_cubes_texture(&self) -> Option<&wgpu::Texture> {
        self.shadows.as_ref().map(|s| s.cubes_texture())
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

    /// The primary view's motion vectors (#481), when it runs the R64
    /// path. `None` on the fallback, which has no vbuf to reconstruct
    /// from.
    pub fn motion_vector_texture(&self) -> Option<&wgpu::Texture> {
        self.views[self.primary]
            .vbuf64_stage
            .as_ref()
            .map(|stage| stage.motion_vector_texture())
    }

    /// The primary view's most recent temporal resolve (#481), for a
    /// test to read back. `None` on the R32 fallback.
    pub fn resolved_texture(&self) -> Option<&wgpu::Texture> {
        self.views[self.primary]
            .vbuf64_stage
            .as_ref()
            .map(|stage| stage.resolved_texture())
    }

    /// Switches temporal anti-aliasing on or off across every view
    /// (#481), which is also what switches the sub-pixel jitter.
    ///
    /// 🔴 Returns how many views took it, for the same reason
    /// [`Self::set_compute_shading`] does: zero means every view is on
    /// the R32 fallback, where there is neither a motion vector nor a
    /// history and this does nothing — and "did nothing" is
    /// indistinguishable from "worked" in anything that only looks at
    /// the image.
    pub fn set_temporal_aa(&mut self, on: bool) -> usize {
        let mut switched = 0;
        for (_, view) in self.views.iter_mut() {
            if let Some(stage) = view.vbuf64_stage.as_mut() {
                stage.set_temporal_aa(on);
                switched += 1;
            }
        }
        switched
    }

    /// Selects the temporal technique on every view that has the R64
    /// stage, and returns how many took it (#536).
    pub fn set_upscale(&mut self, technique: crate::quality::UpscaleTechnique) -> usize {
        self.upscale_technique = technique;
        let mut applied = 0;
        for (_, view) in self.views.iter_mut() {
            if let Some(stage) = view.vbuf64_stage.as_mut() {
                stage.set_upscale(technique);
                applied += 1;
            }
        }
        applied
    }

    /// How hard RCAS sharpens the finished image on every view that has
    /// the R64 stage, 0..=100 (#481 step 5). Returns how many took it.
    pub fn set_sharpening(&mut self, percent: u32) -> usize {
        let mut applied = 0;
        for (_, view) in self.views.iter_mut() {
            if let Some(stage) = view.vbuf64_stage.as_mut() {
                stage.set_sharpening(percent);
                applied += 1;
            }
        }
        applied
    }

    /// How much smaller than its panel each view renders, 1..=100.
    ///
    /// Takes effect on the next `resize_view`, which the editor calls
    /// every frame with the panel's size — so a change lands within a
    /// frame without a reallocation path of its own.
    pub fn set_render_scale(&mut self, scale: u32) {
        self.render_scale = scale.clamp(1, 100);
    }

    /// Whether any view shades in compute.
    fn compute_shading_active(&self) -> bool {
        self.views
            .iter()
            .filter_map(|(_, view)| view.vbuf64_stage.as_ref())
            .any(|stage| stage.compute_shading())
    }

    /// What a fragment coordinate is multiplied by to find its froxel.
    ///
    /// 🔴 Exposed because sizing this from the wrong resolution shipped
    /// (#481 step 4). The grid's DIMENSIONS come from the aspect ratio
    /// and a fixed cluster budget, so they do not move with the
    /// resolution and cannot catch the mistake — this is the number that
    /// does. Built from the window while the shading pass produces
    /// fragment coordinates at render resolution, every pixel reads a
    /// froxel at twice its address. The owner found it by eye; nothing
    /// in the suite could have.
    pub fn cluster_tile_factors(&self) -> glam::Vec2 {
        self.lights.clusters().grid().tile_factors
    }

    /// What a view of `output` renders at, under the current technique.
    pub(super) fn render_size_for(&self, output: (u32, u32)) -> (u32, u32) {
        // 🔴 The fragment path renders at the window's size whatever the
        // settings say. It tonemaps inline into the image the window
        // presents — no HDR target, nothing at render resolution — so a
        // smaller frame there puts a render-sized depth buffer and a
        // window-sized colour target into one pass, and **wgpu discards
        // the pass**. The failure is not a soft picture, it is no
        // picture. Reported from the editor at 1023x816 and 50 %.
        //
        // Applied HERE rather than in the setters, so the order the two
        // arrive in cannot decide the outcome: a scale set before the
        // compute path is switched on must not be lost, and one set
        // after must not slip through.
        if !self.compute_shading_active() {
            return output;
        }
        self.upscale_technique
            .render_size(output, self.render_scale)
    }

    /// The lens both the cull and SGSR 2's edge mask are derived from.
    pub fn set_camera_lens(&mut self, fov_y_rad: f32, aspect: f32) -> usize {
        let mut applied = 0;
        for (_, view) in self.views.iter_mut() {
            if let Some(stage) = view.vbuf64_stage.as_mut() {
                stage.set_camera_lens(fov_y_rad, aspect);
                applied += 1;
            }
        }
        applied
    }

    /// Switches every view between the fragment shading path and the
    /// compute one (#824), overriding `KOOCH_COMPUTE_SHADING`.
    ///
    /// Both pipelines are already built, so this costs nothing to flip
    /// and takes effect on the next frame. It exists because the two
    /// paths have to be compared — a test rendering the same scene twice
    /// cannot do it through a `OnceLock` that reads the environment once
    /// per process, and neither can a live control.
    ///
    /// A view without the R64 stage (no 64-bit texture atomics) has no
    /// compute shading path to switch to and is left alone.
    ///
    /// 🔴 Returns how many views it reached, and a caller that needs the
    /// switch to have happened must check it. Zero means every view is
    /// on the R32 fallback, where this setting does nothing at all —
    /// which is indistinguishable from "the setting worked" in anything
    /// that only looks at the rendered image. #824's parity tests passed
    /// against a stage built with `Vbuf64Support::from_supported(false)`
    /// until they started checking this.
    pub fn set_compute_shading(&mut self, on: bool) -> usize {
        let mut switched = 0;
        for (_, view) in self.views.iter_mut() {
            if let Some(stage) = view.vbuf64_stage.as_mut() {
                stage.set_compute_shading(on);
                switched += 1;
            }
        }
        switched
    }

    /// How many pixels share one shaded sample (#825), across every
    /// view.
    ///
    /// 🔴 Returns how many views took it, for the same reason
    /// [`Self::set_compute_shading`] does — and one more: a reduced rate
    /// needs the compute shading path, so this returns zero both when a
    /// view has no R64 stage and when it has one still on the fragment
    /// path. Either way nothing changed, and only the return value says
    /// so.
    pub fn set_shading_rate(&mut self, rate: crate::meshlet::ShadingRate) -> usize {
        let mut switched = 0;
        for (_, view) in self.views.iter_mut() {
            if let Some(stage) = view.vbuf64_stage.as_mut()
                && stage.set_shading_rate(rate)
            {
                switched += 1;
            }
        }
        switched
    }

    /// The primary view's current shading rate (#825). `Full` when the
    /// view has no R64 stage, which is the rate it renders at.
    pub fn shading_rate(&self) -> crate::meshlet::ShadingRate {
        self.views[self.primary]
            .vbuf64_stage
            .as_ref()
            .map(|s| s.shading_rate())
            .unwrap_or_default()
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
        let render_size = self.render_size_for(size);
        self.views.insert(super::view_targets::MeshletView::new(
            device,
            size,
            render_size,
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

    /// Colour TEXTURE of `id`, or `None` if the handle is stale.
    ///
    /// The view above is what a blit binds; this is what a readback
    /// copies from. Added because every shadow picture this repo takes
    /// came from the primary view, so the Game panel — a second `ViewId`
    /// on the same stage — was the one surface no test could look at.
    pub fn view_color_texture(&self, id: ViewId) -> Option<&wgpu::Texture> {
        self.views.get(id).map(|v| &v.color_texture)
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
