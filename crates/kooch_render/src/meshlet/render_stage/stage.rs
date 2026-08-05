use std::sync::Arc;

use super::super::deferred::MeshletDeferredShader;
use super::super::dispatcher::{MeshletCull, MeshletCullPipelines};
use super::super::gpu_timers::MeshletGpuTimers;
use super::super::pool::GpuGlobalMeshPool;
use super::super::reject_overlay::MeshletRejectOverlay;
use super::super::scene::MeshletScene;
use super::super::stage_counters::MeshletStageCounters;
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
    /// This view's cull buffers. Moves inside the view collection
    /// once a stage carries more than one (#592) — the split that
    /// made that possible is the one below.
    pub(super) cull: MeshletCull,
    /// Cull pipelines + bind group layouts, shared by every view.
    /// Nine compute pipelines per camera is what this avoids.
    pub(super) cull_pipelines: MeshletCullPipelines,
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

    /// This stage's single view.
    ///
    /// A field rather than the fields themselves: everything in it is
    /// per view, everything outside it is shared, and #592 turns this
    /// into a collection. Keeping the boundary explicit now is what
    /// makes that a data change instead of a hunt through 1800 lines.
    pub(super) view: super::view_targets::MeshletViewTargets,

    /// Reject-reason overlay compute pipeline (#454.4). `Some` only
    /// when `MeshletDebugCaps::supports_texture_atomic` is true — the
    /// same gate the density / overdraw modes ride.
    ///
    /// Shared rather than per view: it is a pipeline, and the texture
    /// it writes through comes from whichever view is being rendered.
    pub(super) reject_overlay: Option<MeshletRejectOverlay>,

    pub(super) instance_capacity: u32,

    pub(super) frame_bind_groups: [Vec<wgpu::BindGroup>; 3],
    /// Round-robin index for `frame_bind_groups`.
    #[allow(dead_code)]
    pub(super) frame_bind_groups_index: usize,

    /// GPU frame timing via wgpu timestamp queries. Disabled by
    /// default (see [`Self::enable_gpu_timers`]). Tests don't pay
    /// for this; the editor / game runtime opts in at startup.
    pub(super) gpu_timers: MeshletGpuTimers,

    /// Async CPU mirror of the cull pipeline's per-stage survivor
    /// counters (#454.6). Allocated unconditionally — the GPU
    /// footprint is 48 B and the ring stays idle when no
    /// debug-active mode is selected.
    pub(super) stage_counters: MeshletStageCounters,

    /// Cross-module engine VRAM counter (#463.5). Optional —
    /// `None` means the editor / game has not registered a tracker
    /// and the stage skips bookkeeping. Wired via
    /// [`Self::set_vram_tracker`] at startup.
    pub(super) vram_tracker: Option<Arc<EngineVramTracker>>,
}
