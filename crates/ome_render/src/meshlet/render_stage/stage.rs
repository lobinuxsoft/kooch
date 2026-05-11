use std::sync::Arc;

use super::super::deferred::MeshletDeferredShader;
use super::super::dispatcher::MeshletCull;
use super::super::gpu_timers::MeshletGpuTimers;
use super::super::pool::GpuGlobalMeshPool;
use super::super::scene::MeshletScene;
use super::super::system::MeshletPipeline;
use super::super::vbuf64_stage::Vbuf64Stage;
use super::super::vis_buffer::MeshletVisRasterizer;
use crate::hi_z::HiZ;
use crate::perf::EngineVramTracker;

/// End-to-end meshlet render stage. See module docs for the per-frame
/// flow.
///
/// Material data lives in `MaterialPipeline` (a `Resources` entry
/// owned by the asset plugin) — the stage borrows its bind group at
/// render time. Callers MUST insert a `MaterialPipeline` before
/// calling `render_with_assets`; headless tests construct one with
/// `MaterialPipeline::with_capacity(device, n)` and `register` the
/// materials they need.
pub struct MeshletRenderStage {
    pub(super) pipeline: MeshletPipeline,
    pub(super) scene: MeshletScene,
    pub(super) cull: MeshletCull,
    pub(super) rasterizer: MeshletVisRasterizer,
    pub(super) deferred: MeshletDeferredShader,

    /// GPU mirror of [`MeshletPipeline::pool`]. Lazy-rebuilt by
    /// [`Self::render_with_assets`] when [`Self::pool_dirty`] is set,
    /// which happens whenever [`Self::ensure_gpu_mesh`] introduces a
    /// new GUID. `None` until the first registration.
    pub(super) gpu_pool: Option<GpuGlobalMeshPool>,
    /// `true` when the CPU pool has changed since the last
    /// `gpu_pool` rebuild. Cheap to check before each frame.
    pub(super) pool_dirty: bool,

    pub(super) meshlet_bgl: wgpu::BindGroupLayout,

    /// Atomic R64 visibility-buffer pipeline (#493). `Some` only when
    /// the device supports `TEXTURE_INT64_ATOMIC | SHADER_INT64 |
    /// SHADER_INT64_ATOMIC_MIN_MAX` (see [`Vbuf64Support`]). When set,
    /// the per-frame orchestrator routes the scene draw through it
    /// instead of the legacy R32Uint color-attachment vbuf — fixes the
    /// coplanar-meshlet z-fighting that the legacy path exhibits, with
    /// no functional difference for the rest of the engine.
    pub(super) vbuf64_stage: Option<Vbuf64Stage>,

    pub(super) vbuf_view: wgpu::TextureView,
    pub(super) depth_view: wgpu::TextureView,
    /// Depth-only view of the same depth texture, suitable for
    /// `cs_copy_depth` in the Hi-Z builder. Sampling-bind requires
    /// `TextureAspect::DepthOnly` whereas the render attachment uses
    /// `TextureAspect::All`; sharing one view across both roles
    /// would fail wgpu validation in the worst case.
    pub(super) depth_sample_view: wgpu::TextureView,
    pub(super) color_view: wgpu::TextureView,

    pub(super) vbuf_texture: wgpu::Texture,
    pub(super) depth_texture: wgpu::Texture,
    pub(super) color_texture: wgpu::Texture,

    /// Per-pixel R32Uint atomic accumulator (#454) backing the
    /// `TriangleDensity` / `Overdraw` heatmap modes and the reject
    /// overlay raster pass. `Some` only when the device exposes
    /// `Features::TEXTURE_ATOMIC` (mirrored through
    /// [`MeshletDebugCaps`]); otherwise stays `None` and the dropdown
    /// filter never lets the user pick a mode that would read it.
    /// Resized in lock-step with the vbuf / color targets and cleared
    /// to zero before every raster pass that writes through it.
    pub(super) triangle_density_texture: Option<wgpu::Texture>,
    pub(super) triangle_density_view: Option<wgpu::TextureView>,

    pub(super) size: (u32, u32),
    pub(super) instance_capacity: u32,

    /// Twin Hi-Z pyramids for the 2-pass cull (#445). Pass A samples
    /// `hiz_prev` (last frame's depth, may have false negatives on
    /// newly-revealed geometry); pass B rebuilds `hiz_curr` from the
    /// pass-A raster's depth and re-tests the pass-A rejects to
    /// recover anything that became visible this frame. At the end of
    /// the frame the orchestrator swaps `hiz_prev <- hiz_curr` so the
    /// next frame's pass A reads the freshest pyramid we have.
    ///
    /// Lazy Hi-Z pyramids. The 2-pass orchestrator that samples
    /// these is parked behind the SPD follow-up (#486); the current
    /// single-pass orchestrator never reads them, so allocating them
    /// at `new()` time wastes VRAM and surfaces wgpu validation
    /// noise from the editor's per-frame placeholder stage. They
    /// stay `None` until the SPD-backed orchestrator switches them
    /// on via `ensure_hi_z_pyramids()`.
    pub(super) hiz_prev: Option<HiZ>,
    pub(super) hiz_curr: Option<HiZ>,
    /// `false` until the orchestrator has called `clear_to_far` on
    /// the freshly-created `hiz_prev`. Reset to `false` on `resize()`
    /// because both pyramids are recreated and need re-init. The
    /// next call to `render_with_assets` after that bump runs the
    /// init upload so pass A samples a "nothing occluded" pyramid.
    pub(super) hi_z_initialized: bool,
    /// Triple-buffered per-frame arena reserved for the future SPD
    /// follow-up that activates the Hi-Z 2-pass orchestrator. The
    /// orchestrator currently uses `dispatch_scene_pool_atomic`
    /// (no Hi-Z), so the arena stays empty in production. When the
    /// SPD-backed pyramid build lands and the orchestrator switches
    /// to `dispatch_scene_pool_atomic_hi_z` + `dispatch_cull_pass_b`,
    /// each per-frame bind group parks here so it outlives the GPU's
    /// use of it (Mesa radv invalidates bind groups dropped while
    /// in flight). Three slots ≥ wgpu's max in-flight-frames headroom.
    #[allow(dead_code)]
    pub(super) frame_bind_groups: [Vec<wgpu::BindGroup>; 3],
    /// Round-robin index for `frame_bind_groups`.
    #[allow(dead_code)]
    pub(super) frame_bind_groups_index: usize,
    /// Pyramids retired by `resize()` that may still be in flight on
    /// the GPU. Triple-buffered to defer the drop until after the
    /// GPU has stopped using the views — same Mesa radv lifetime
    /// rule as `frame_bind_groups`. Currently no-op since the
    /// orchestrator doesn't sample the pyramids.
    #[allow(dead_code)]
    pub(super) retired_pyramids: [Vec<HiZ>; 3],

    /// GPU frame timing via wgpu timestamp queries. Disabled by
    /// default (see [`Self::enable_gpu_timers`]). Tests don't pay
    /// for this; the editor / game runtime opts in at startup.
    pub(super) gpu_timers: MeshletGpuTimers,

    /// Cross-module engine VRAM counter (#463.5). Optional —
    /// `None` means the editor / game has not registered a tracker
    /// and the stage skips bookkeeping. Wired via
    /// [`Self::set_vram_tracker`] at startup.
    pub(super) vram_tracker: Option<Arc<EngineVramTracker>>,
}
