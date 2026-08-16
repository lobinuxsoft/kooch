use std::sync::Arc;

slotmap::new_key_type! {
    /// Handle to one view inside a [`MeshletRenderStage`]. Returned by
    /// `create_view`; stale ids read as `None` rather than as another
    /// view.
    pub struct ViewId;
}

use super::super::deferred::MeshletDeferredShader;
use super::super::dispatcher::MeshletCullPipelines;
use super::super::gpu_timers::MeshletGpuTimers;
use super::super::pool::GpuGlobalMeshPool;
use super::super::reject_overlay::MeshletRejectOverlay;
use super::super::scene::MeshletScene;
use super::super::stage_counters::MeshletStageCounters;
use super::super::system::MeshletPipeline;
use super::super::vbuf64_stage::Vbuf64Stage;
use super::super::vis_buffer::MeshletVisRasterizer;
use super::config::MeshletRenderStageConfig;
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
    /// Cull pipelines + bind group layouts, shared by every view.
    /// Nine compute pipelines per camera is what this avoids.
    pub(super) cull_pipelines: MeshletCullPipelines,
    pub(super) rasterizer: MeshletVisRasterizer,
    pub(super) deferred: MeshletDeferredShader,

    /// Inti's GPU residency: the frame constants + the light storage
    /// buffer, refreshed once per `render` call.
    ///
    /// Shared across views rather than per view, because which lights
    /// exist does not depend on where a camera is. The one per-view
    /// value it carries — the camera position — is safe here only
    /// because each view records *and submits* its own encoder; see
    /// [`kooch_lighting::GpuLights`] for the ordering argument.
    pub(super) lights: kooch_lighting::GpuLights,

    /// The sun's shadow atlas and depth pipeline (#476).
    ///
    /// `None` until a frame finds a directional light that casts, and
    /// allocated once when one does: the atlas is 64 MiB at the default
    /// resolution, and a headless test, an unlit scene or a project that
    /// turned shadows off must not pay it. Dropped again when the
    /// author disables shadows or changes the cascade resolution — the
    /// texture size is baked in at allocation.
    ///
    /// Shared across views for the same reason the lights are: which
    /// geometry occludes the sun does not depend on where a camera is.
    /// The cascade *placement* does, and it is per frame rather than
    /// stored — each view rebuilds it from its own camera before
    /// recording, and each view submits its own encoder.
    pub(super) shadows: Option<crate::shadow::ShadowPass>,
    /// Cascade resolution `shadows` was allocated at, so a settings
    /// change is noticed rather than silently ignored.
    pub(super) shadow_texels: u32,
    /// Whether any casting point light went without a cube last frame
    /// (#778), so the warning fires on the transition rather than sixty
    /// times a second. Same shape as the light-count log.
    ///
    /// 🔴 A **flag**, not the count. It was the count, and the count is
    /// not steady: the lights are culled against the frustum before the
    /// budget is applied, so nudging the camera moves how many casters
    /// are visible — measured swinging between 84 and 96 in a scene of
    /// 100 — and "log when it changes" then logs every single frame.
    /// What an author needs to know is that the budget is exceeded at
    /// all; the exact overflow changes with where they are standing and
    /// says nothing more.
    pub(super) point_shadows_over_budget: bool,
    /// The lights that held a cube last frame, so
    /// [`select_point_casters`](crate::shadow::select_point_casters) can
    /// favour them over a rival that is barely ahead. At most
    /// `MAX_POINT_SHADOWS` entries — this is a hysteresis term, not a
    /// cache.
    pub(super) point_shadow_holders: Vec<kooch_ecs::entity::Entity>,
    /// Hash of every instance uploaded this frame, and the cached cube
    /// key per point-shadow slot (#778). Together they answer "may last
    /// frame's six faces stand".
    /// One entry per instance this frame — see
    /// [`InstanceBounds`](crate::shadow::InstanceBounds). Replaces the
    /// single scene-wide hash the cube cache used to key on (#847).
    pub(super) instance_bounds: Vec<crate::shadow::InstanceBounds>,
    pub(super) point_cube_cache: Vec<Option<crate::shadow::CubeKey>>,

    /// GPU mirror of [`MeshletPipeline::pool`]. Lazy-rebuilt by
    /// [`Self::render_with_assets`] when [`Self::pool_dirty`] is set,
    /// which happens whenever [`Self::ensure_gpu_mesh`] introduces a
    /// new GUID. `None` until the first registration.
    pub(super) gpu_pool: Option<GpuGlobalMeshPool>,
    /// `true` when the CPU pool has changed since the last
    /// `gpu_pool` rebuild. Cheap to check before each frame.
    pub(super) pool_dirty: bool,

    pub(super) meshlet_bgl: wgpu::BindGroupLayout,

    /// This stage's views. A frame is a *list* of them (#592): the
    /// game surface, the editor viewport, a camera rendering into a
    /// texture, and later one per shadow cascade and per Virtual Shadow
    /// Map page. What multiplies is the view — the geometry pool above
    /// stays singular.
    ///
    /// A generational key rather than a `Vec` index: closing an editor
    /// panel leaves whoever held its id holding a stale one, and a bare
    /// index would silently start addressing a different view.
    pub(super) views: slotmap::SlotMap<ViewId, super::view_targets::MeshletView>,
    /// The view the single-view accessors (`color_view`, `size`, …)
    /// read. Every stage has at least one; callers that own more than
    /// one address them by [`ViewId`] instead.
    pub(super) primary: ViewId,
    /// What this stage was built with, so a view added later gets the
    /// same capability gates rather than whatever the caller happened
    /// to reconstruct.
    pub(super) config: MeshletRenderStageConfig,

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

    /// Frames this stage has recorded, for anything that wants a
    /// temporally varying value. Today that is the contact-shadow
    /// jitter (#735): without it the dither pattern is frozen into the
    /// image and reads as a texture rather than as noise.
    pub(super) frames_recorded: u32,

    /// Cross-module engine VRAM counter (#463.5). Optional —
    /// `None` means the editor / game has not registered a tracker
    /// and the stage skips bookkeeping. Wired via
    /// [`Self::set_vram_tracker`] at startup.
    pub(super) vram_tracker: Option<Arc<EngineVramTracker>>,
}
