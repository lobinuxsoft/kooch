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

use super::deferred::{MeshletDeferredShader, DEFERRED_COLOR_FORMAT};
use super::dispatcher::MeshletCull;
use super::gpu_meshlet::meshlet_bind_group_layout;
use super::pool::GpuGlobalMeshPool;
use super::scene::MeshletScene;
use super::system::MeshletPipeline;
use super::vis_buffer::{MeshletVisRasterizer, VISIBILITY_BUFFER_FORMAT};
use super::DEFAULT_MAX_TRIANGLES;

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
    pub(super) color_view: wgpu::TextureView,

    pub(super) vbuf_texture: wgpu::Texture,
    pub(super) depth_texture: wgpu::Texture,
    pub(super) color_texture: wgpu::Texture,

    pub(super) size: (u32, u32),
    pub(super) instance_capacity: u32,
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
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let (color_texture, color_view) = create_2d_attachment(
            device,
            "meshlet_render_stage_color",
            size,
            DEFERRED_COLOR_FORMAT,
            wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );

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
            color_view,
            vbuf_texture,
            depth_texture,
            color_texture,
            size,
            instance_capacity,
        }
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
}
