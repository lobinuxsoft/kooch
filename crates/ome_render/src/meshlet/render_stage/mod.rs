//! End-to-end meshlet render stage — Phase 1.E.3c orchestrator.
//!
//! Owns the full per-frame meshlet pipeline state and runs the
//! cull → vbuf → deferred chain off ECS data:
//!
//! ```text
//! Resources (MeshRenderer + GlobalTransform + Assets<MeshletMesh>)
//!         │  collect_scene_instances
//!         ▼
//! Vec<MeshInstance>  ──► MeshletScene.upload_instances
//!         │
//!         ▼
//! MeshletCull.dispatch_scene_pool  (one dispatch over the entire GpuGlobalMeshPool)
//!         │
//!         ▼
//! MeshletVisRasterizer.render_scene  (R32Uint visibility buffer + depth)
//!         │
//!         ▼
//! MeshletDeferredShader.shade_scene  (compute → Rgba8Unorm color view)
//! ```
//!
//! # Multi-mesh path (#446 / #457)
//!
//! [`Self::ensure_gpu_mesh`] registers a `MeshletMesh` into the
//! [`MeshletPipeline`]'s `GlobalMeshPool` and marks the GPU mirror
//! dirty. [`Self::render_with_assets`] rebuilds the
//! [`GpuGlobalMeshPool`] when dirty, then dispatches
//! `cs_cull_scene_pool` over every (instance, meshlet) pair across
//! the whole pool — one cull dispatch per frame regardless of how
//! many distinct meshes the scene references.
//!
//! # Owning vs borrowing
//!
//! The stage *owns* the visibility buffer / depth / color textures so
//! the same allocations survive across frames. The plugin layer
//! (1.E.3b) will hand the color view back to the editor's offscreen
//! target via a copy or by binding the stage's view directly.

mod frame;

use crate::material::{MaterialParams, MaterialPool};

use std::sync::Arc;

use super::deferred::{MeshletDeferredShader, DEFERRED_COLOR_FORMAT};
use super::dispatcher::MeshletCull;
use super::gpu_meshlet::meshlet_bind_group_layout;
use super::gpu_timers::MeshletGpuTimers;
use super::pool::GpuGlobalMeshPool;
use super::scene::MeshletScene;
use super::system::MeshletPipeline;
use super::vis_buffer::{MeshletVisRasterizer, VISIBILITY_BUFFER_FORMAT};
use super::DEFAULT_MAX_TRIANGLES;
use crate::hi_z::HiZ;
use crate::perf::EngineVramTracker;

/// Construction parameters for [`MeshletRenderStage`]. All sizes are
/// upper bounds — the actual per-frame instance count comes from the
/// ECS query.
pub struct MeshletRenderStageConfig {
    /// Output color / depth / vbuf resolution. Must be > 0 in both axes.
    pub size: (u32, u32),
    /// Maximum number of [`MeshInstance`](super::scene::MeshInstance)
    /// records the scene buffer can hold per frame.
    pub instance_capacity: u32,
    /// Capacity (in surviving meshlet slots) of the cull dispatcher's
    /// `visible_meshlets` storage. For the scene path, set this to at
    /// least `instance_capacity * meshlets_per_mesh` so no thread loses
    /// its slot to atomic-overflow.
    pub meshlet_capacity: u32,
    /// Initial material pool. Must be non-empty (wgpu rejects
    /// zero-sized storage buffer bindings).
    pub materials: Vec<MaterialParams>,
}

impl Default for MeshletRenderStageConfig {
    fn default() -> Self {
        Self {
            size: (256, 256),
            instance_capacity: 256,
            meshlet_capacity: 4096,
            materials: vec![MaterialParams::default()],
        }
    }
}

/// Per-frame return value reporting how the stage spent its budget.
/// Surfaced through the editor's debug-stats overlay (#451) and used
/// by the integration test as a render side-effect.
///
/// Per-stage cull survivor counts (frustum / backface / hi-z) require
/// a 4-byte CPU readback per frame and ship in #451b together with
/// the reject-reason tagging buffer.
#[derive(Copy, Clone, Debug, Default)]
pub struct MeshletRenderStats {
    /// Number of `MeshInstance` records uploaded this frame.
    pub instances_uploaded: u32,
    /// Worst-case `(instance_count × meshlets_per_mesh)` thread budget
    /// the cull dispatch saw — equals the upper bound on surviving
    /// meshlets before any cull rejection.
    pub cull_threads: u32,
    /// World-space camera position the cull / LOD selector saw this
    /// frame. Surfaced in the View toolbar so the artist can confirm
    /// the selector is actually following the active camera (the LOD
    /// boundary rule is distance-driven; if this stays static while
    /// the editor camera moves, the selector is reading the wrong
    /// view matrix).
    pub cam_pos: [f32; 3],
    /// Total meshlet count across the entire `GlobalMeshPool` (every
    /// LOD of every registered mesh, concatenated).
    pub pool_meshlets_total: u32,
    /// Subset of `pool_meshlets_total` whose `parent_meshlet_index`
    /// is the sentinel — terminal stops for the runtime selector.
    /// `roots == total` ⇒ the chain has no usable depth (every mesh
    /// is single-LOD or every group failed to simplify). `roots <<
    /// total` ⇒ the chain has depth and the selector should be able
    /// to descend / ascend across distance.
    pub pool_meshlets_roots: u32,
    /// Wall-clock duration of the cull → vbuf raster → deferred
    /// shade chain on the GPU, in milliseconds. `None` when GPU
    /// timers are disabled (no `Features::TIMESTAMP_QUERY` support
    /// or `enable_gpu_timers` was never called) or the first ring
    /// readback hasn't landed yet (1-2 frames after enable).
    pub gpu_frame_ms: Option<f32>,
    /// Number of dispatch / render-pass operations emitted by the
    /// meshlet pipeline this frame (#463.6). Indirect dispatch
    /// means this is bounded — one cull, one vbuf raster, one
    /// deferred shade — regardless of instance count. Sky / gizmo /
    /// blit / egui passes outside the meshlet stage are counted
    /// separately by the editor render system.
    pub draw_calls: u32,
}

/// End-to-end meshlet render stage. See module docs for the per-frame
/// flow.
pub struct MeshletRenderStage {
    pub(super) pipeline: MeshletPipeline,
    pub(super) scene: MeshletScene,
    pub(super) cull: MeshletCull,
    pub(super) rasterizer: MeshletVisRasterizer,
    pub(super) deferred: MeshletDeferredShader,
    pub(super) material_pool: MaterialPool,

    /// GPU mirror of [`MeshletPipeline::pool`]. Lazy-rebuilt by
    /// [`Self::render_with_assets`] when [`Self::pool_dirty`] is set,
    /// which happens whenever [`Self::ensure_gpu_mesh`] introduces a
    /// new GUID. `None` until the first registration.
    pub(super) gpu_pool: Option<GpuGlobalMeshPool>,
    /// `true` when the CPU pool has changed since the last
    /// `gpu_pool` rebuild. Cheap to check before each frame.
    pub(super) pool_dirty: bool,

    pub(super) meshlet_bgl: wgpu::BindGroupLayout,

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
    /// First-frame: `hiz_prev` is initialised to 1.0 (far depth) on
    /// the very first call to `render_with_assets` so pass A's
    /// conservative Hi-Z reject keeps every meshlet, the raster
    /// fills depth, pass B builds `hiz_curr` and re-tests an empty
    /// reject set. From frame 2 onward both pyramids carry signal.
    pub(super) hiz_prev: HiZ,
    pub(super) hiz_curr: HiZ,
    /// `false` until the orchestrator has called `clear_to_far` on
    /// the freshly-created `hiz_prev`. Reset to `false` on `resize()`
    /// because both pyramids are recreated and need re-init. The
    /// next call to `render_with_assets` after that bump runs the
    /// init upload so pass A samples a "nothing occluded" pyramid.
    pub(super) hi_z_initialized: bool,

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

impl MeshletRenderStage {
    pub fn new(device: &wgpu::Device, config: MeshletRenderStageConfig) -> Self {
        let MeshletRenderStageConfig {
            size,
            instance_capacity,
            meshlet_capacity,
            materials,
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
        let material_pool = MaterialPool::new(device, &materials);

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

        // Twin Hi-Z pyramids match the depth attachment dimensions.
        // resize() recreates both whenever `size` changes.
        let hiz_prev = HiZ::new(device, size.0, size.1);
        let hiz_curr = HiZ::new(device, size.0, size.1);

        Self {
            pipeline: MeshletPipeline::new(),
            scene,
            cull,
            rasterizer,
            deferred,
            material_pool,
            gpu_pool: None,
            pool_dirty: false,
            meshlet_bgl,
            vbuf_view,
            depth_view,
            depth_sample_view,
            color_view,
            vbuf_texture,
            depth_texture,
            color_texture,
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
        }
    }

    /// Swaps the current Hi-Z pyramid into the `prev` slot. Called by
    /// [`Self::render_with_assets`] at the end of every frame so the
    /// next frame's pass A reads the pyramid this frame just built.
    pub(super) fn swap_hi_z_pyramids(&mut self) {
        std::mem::swap(&mut self.hiz_prev, &mut self.hiz_curr);
    }

    /// Read-only access to the pyramid pass A samples this frame.
    /// Mainly here so integration tests can confirm the swap
    /// happened — the orchestrator binds it through field access.
    pub fn hi_z_prev(&self) -> &HiZ {
        &self.hiz_prev
    }

    /// Read-only access to the pyramid pass B samples (= the one
    /// rebuilt from this frame's depth between cull A and cull B).
    pub fn hi_z_curr(&self) -> &HiZ {
        &self.hiz_curr
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
        // in `new()` — vbuf, depth, color, plus the twin Hi-Z
        // pyramids. Any tracker setup AFTER construction must still
        // see those bytes.
        let attachment_bytes = render_target_byte_estimate(self.size);
        let pyramid_bytes = self.hiz_prev.byte_size() + self.hiz_curr.byte_size();
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

    pub fn material_pool(&self) -> &MaterialPool {
        &self.material_pool
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

/// Approximate bytes occupied by the stage's three render targets at
/// the given resolution. Used by [`MeshletRenderStage::set_vram_tracker`]
/// to seed the counter with what `new()` already allocated.
fn render_target_byte_estimate(size: (u32, u32)) -> u64 {
    let pixels = size.0 as u64 * size.1 as u64;
    // vbuf: R32Uint = 4 bpp; depth: Depth32Float = 4 bpp;
    // color: Rgba8Unorm = 4 bpp. Total: 12 bytes/pixel.
    pixels * 12
}

/// Creates a `TextureAspect::DepthOnly` view of a depth attachment so
/// the same texture can be sampled by the Hi-Z builder while the
/// `_view` (with `aspect: All`) drives the render pass attachment.
pub(super) fn depth_sample_view(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("meshlet_render_stage_depth_sample"),
        format: Some(wgpu::TextureFormat::Depth32Float),
        dimension: Some(wgpu::TextureViewDimension::D2),
        usage: None,
        aspect: wgpu::TextureAspect::DepthOnly,
        base_mip_level: 0,
        mip_level_count: Some(1),
        base_array_layer: 0,
        array_layer_count: Some(1),
    })
}

pub(super) fn create_2d_attachment(
    device: &wgpu::Device,
    label: &str,
    size: (u32, u32),
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_are_non_zero() {
        let cfg = MeshletRenderStageConfig::default();
        assert!(cfg.size.0 > 0 && cfg.size.1 > 0);
        assert!(cfg.instance_capacity > 0);
        assert!(cfg.meshlet_capacity > 0);
        assert!(!cfg.materials.is_empty());
    }

    #[test]
    fn stats_default_is_zero() {
        let s = MeshletRenderStats::default();
        assert_eq!(s.instances_uploaded, 0);
        assert_eq!(s.cull_threads, 0);
    }

    #[test]
    fn hi_z_byte_size_matches_summed_mips() {
        // Pure-CPU verification of the pyramid-byte arithmetic that
        // set_vram_tracker / resize rely on. R32Float = 4 bpp summed
        // over the mip chain — for a square power-of-two pyramid this
        // converges to base * 4/3 (geometric series factor 1/4).
        // 64x64 → 7 mips, exact total = (4096+1024+256+64+16+4+1) * 4 = 21844.
        let expected = (4096 + 1024 + 256 + 64 + 16 + 4 + 1) * 4u64;
        let mip_count = crate::hi_z::mip_count_for(64, 64);
        let mut total: u64 = 0;
        for level in 0..mip_count {
            let (w, h) = crate::hi_z::mip_size(64, 64, level);
            total += (w as u64) * (h as u64) * 4;
        }
        assert_eq!(total, expected);
    }
}
