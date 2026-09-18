use std::collections::HashSet;
use std::sync::Arc;

use kooch_core::Guid;

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
use super::super::vis_buffer::MeshletVisRasterizer;
use super::config::MeshletRenderStageConfig;
use crate::perf::EngineVramTracker;

/// End-to-end meshlet render stage. See module docs for the per-frame flow.
pub struct MeshletRenderStage {
    pub(super) pipeline: MeshletPipeline,
    pub(super) scene: MeshletScene,
    /// Cull pipelines + bind group layouts, shared by every view.
    /// Nine compute pipelines per camera is what this avoids.
    pub(super) cull_pipelines: MeshletCullPipelines,
    pub(super) rasterizer: MeshletVisRasterizer,
    pub(super) deferred: MeshletDeferredShader,

    /// Inti's GPU residency: the frame constants + the light storage buffer, refreshed once per
    /// `render` call.
    pub(super) lights: kooch_lighting::GpuLights,
    /// The light walk of the current frame, and the frame it was taken in.
    pub(super) light_frame: Option<(u64, kooch_lighting::LightFrame)>,

    /// The sun's shadow atlas and depth pipeline (#476).
    pub(super) shadows: Option<crate::shadow::ShadowPass>,
    /// Cascade resolution `shadows` was allocated at, so a settings change is noticed rather than
    /// silently ignored. What the classic shadow pass holds allocated, or zeroed when it holds
    /// nothing. The key the resize-release compares (#945).
    pub(super) shadow_alloc: super::frame::ClassicAlloc,
    /// Whether any casting point light went without a cube last frame (#778), so the warning fires
    /// on the transition rather than sixty times a second. Same shape as the light-count log.
    pub(super) point_shadows_over_budget: bool,
    /// The lights that held a cube last frame, so
    /// [`select_point_casters`](crate::shadow::select_point_casters) can favour them over a rival
    /// that is barely ahead.
    pub(super) point_shadow_holders: Vec<kooch_ecs::entity::Entity>,
    /// What decides how much smaller than its panel a view renders (#481 step 4). Kept on the stage
    /// rather than asked of the settings at allocation time, because a view is resized by the
    /// editor dragging a divider — which knows the panel's size and nothing about upscaling.
    pub(super) upscale_technique: crate::quality::UpscaleTechnique,
    pub(super) render_scale: u32,
    /// Hash of every instance uploaded this frame, and the cached cube key per point-shadow slot
    /// (#778). Together they answer "may last frame's six faces stand". One entry per instance this
    /// frame — see [`InstanceBounds`](crate::shadow::InstanceBounds).
    pub(super) instance_bounds: Vec<crate::shadow::InstanceBounds>,
    /// Last frame's [`Self::instance_bounds`], for the page cache's
    /// movement diff (#477): a caster whose hash changed invalidates
    /// the shadow pages both its old and its new bounds reach.
    pub(super) previous_bounds: Vec<crate::shadow::InstanceBounds>,
    /// This frame's moved-caster spheres, old and new bounds alike,
    /// rebuilt where `instance_bounds` is.
    pub(super) moved_casters: Vec<[f32; 4]>,
    pub(super) point_cube_cache: Vec<Option<crate::shadow::CubeKey>>,

    /// GPU mirror of [`MeshletPipeline::pool`]. Lazy-rebuilt by [`Self::render_with_assets`] when
    /// [`Self::pool_dirty`] is set, which happens whenever [`Self::ensure_gpu_mesh`] introduces a
    /// new GUID. `None` until the first registration.
    pub(super) gpu_pool: Option<GpuGlobalMeshPool>,
    /// `true` when the CPU pool has changed since the last
    /// `gpu_pool` rebuild. Cheap to check before each frame.
    pub(super) pool_dirty: bool,

    /// Mesh GUIDs whose load already failed and was already said out loud.
    pub(super) unresolved: HashSet<Guid>,

    pub(super) meshlet_bgl: wgpu::BindGroupLayout,

    /// This stage's views. A frame is a *list* of them (#592): the game surface, the editor
    /// viewport, a camera rendering into a texture, and later one per shadow cascade and per
    /// Virtual Shadow Map page.
    pub(super) views: slotmap::SlotMap<ViewId, super::view_targets::MeshletView>,
    /// The view the single-view accessors (`color_view`, `size`, …)
    /// read. Every stage has at least one; callers that own more than
    /// one address them by [`ViewId`] instead.
    pub(super) primary: ViewId,
    /// What this stage was built with, so a view added later gets the
    /// same capability gates rather than whatever the caller happened
    /// to reconstruct.
    pub(super) config: MeshletRenderStageConfig,

    /// Reject-reason overlay compute pipeline (#454.4). `Some` only when
    /// `MeshletDebugCaps::supports_texture_atomic` is true — the same gate the density / overdraw
    /// modes ride.
    pub(super) reject_overlay: Option<MeshletRejectOverlay>,
    /// The shadow-page marking pass (#866), when it was asked for.
    pub(super) page_marker: Option<crate::shadow::pages::mark::PageMarker>,
    /// The last count read back, for the panel.
    pub(super) page_marking_last: Option<crate::shadow::pages::mark::MarkCounts>,
    /// The last count LOGGED, per camera.
    pub(super) page_marking_logged: Vec<Option<crate::shadow::pages::mark::MarkCounts>>,
    /// The paged depth raster and its atlas. 🔴 Built with the marker
    /// and never before it: the atlas is a hundred megabytes and it has
    /// nothing to hold until pages are being marked.
    pub(super) page_raster: Option<crate::shadow::pages::raster::PageRasterizer>,
    pub(super) page_raster_last: Option<crate::shadow::pages::raster::RasterCounts>,
    pub(super) page_raster_logged: Vec<Option<crate::shadow::pages::raster::RasterCounts>>,
    /// The pool the atlas was built for. A change rebuilds it.
    pub(super) page_pool_config: Option<crate::shadow::pages::pool::PoolConfig>,
    /// Last frame's shadow casters, for the two questions the page machine asks about them: is
    /// there still ANY, and did one leave.
    pub(super) page_casters: Option<crate::shadow::pages::Casters>,
    /// The scene set the page table was filled for.
    pub(super) page_epoch: Option<u32>,

    pub(super) instance_capacity: u32,

    pub(super) frame_bind_groups: [Vec<wgpu::BindGroup>; 3],
    /// Round-robin index for `frame_bind_groups`.
    #[allow(dead_code)]
    pub(super) frame_bind_groups_index: usize,

    /// GPU frame timing via wgpu timestamp queries. Disabled by
    /// default (see [`Self::enable_gpu_timers`]). Tests don't pay
    /// for this; the editor / game runtime opts in at startup.
    pub(super) gpu_timers: MeshletGpuTimers,

    /// Async CPU mirror of the cull pipeline's per-stage survivor counters (#454.6). Allocated
    /// unconditionally — the GPU footprint is 48 B and the ring stays idle when no debug-active
    /// mode is selected.
    pub(super) stage_counters: MeshletStageCounters,

    /// Frames this stage has recorded, for anything that wants a temporally varying value. Today
    /// that is the contact-shadow jitter (#735): without it the dither pattern is frozen into the
    /// image and reads as a texture rather than as noise.
    pub(super) frames_recorded: u32,

    /// Cross-module engine VRAM counter (#463.5). Optional — `None` means the editor / game has not
    /// registered a tracker and the stage skips bookkeeping. Wired via [`Self::set_vram_tracker`]
    /// at startup.
    pub(super) vram_tracker: Option<Arc<EngineVramTracker>>,
}
