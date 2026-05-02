//! End-to-end meshlet render stage — Phase 1.E.3a orchestrator.
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
//! MeshletCull.dispatch_scene  (compute, one dispatch over instance×meshlet)
//!         │
//!         ▼
//! MeshletVisRasterizer.render_scene  (R32Uint visibility buffer + depth)
//!         │
//!         ▼
//! MeshletDeferredShader.shade_scene  (compute → Rgba8Unorm color view)
//! ```
//!
//! # Single-mesh constraint (1.E.3a)
//!
//! [`Self::render`] takes one [`GpuMeshletMesh`] and dispatches the
//! scene cull against it. Multi-mesh scenes need
//! `cs_cull_scene_pool` + the `GpuGlobalMeshPool`; that's Phase
//! 1.E.3c. Until then every visible entity must reference the same
//! registered `MeshletMesh` (the ECS bridge already enforces
//! "registered" via [`MeshletPipeline::register_mesh`]).
//!
//! # Owning vs borrowing
//!
//! The stage *owns* the visibility buffer / depth / color textures so
//! the same allocations survive across frames. The plugin layer
//! (1.E.3b) will hand the color view back to the editor's offscreen
//! target via a copy or by binding the stage's view directly.

use std::collections::HashMap;

use glam::{Mat4, Vec3};

use ome_core::assets::{Assets, Handle};
use ome_core::resource::Resources;

use crate::material::{MaterialParams, MaterialPool};

use super::asset::MeshletMesh;
use super::cull::CullParams;
use super::deferred::{MeshletDeferredShader, DEFERRED_COLOR_FORMAT};
use super::dispatcher::MeshletCull;
use super::gpu_meshlet::{meshlet_bind_group, meshlet_bind_group_layout, GpuMeshletMesh};
use super::scene::{MeshletScene, SceneCullParams};
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
/// Useful for the integration test — and later for the editor HUD.
#[derive(Copy, Clone, Debug, Default)]
pub struct MeshletRenderStats {
    /// Number of `MeshInstance` records uploaded this frame.
    pub instances_uploaded: u32,
    /// Worst-case `(instance_count × meshlets_per_mesh)` thread budget
    /// the cull dispatch saw — `instance_count * meshlets_per_mesh`.
    pub cull_threads: u32,
}

/// End-to-end meshlet render stage. See module docs for the per-frame
/// flow.
pub struct MeshletRenderStage {
    pipeline: MeshletPipeline,
    scene: MeshletScene,
    cull: MeshletCull,
    rasterizer: MeshletVisRasterizer,
    deferred: MeshletDeferredShader,
    material_pool: MaterialPool,

    /// GPU mirrors of every meshlet mesh that's been registered via
    /// [`Self::ensure_gpu_mesh`]. Single-mesh path (1.E.3a/b) uses the
    /// first cached entry; the multi-mesh `cs_cull_scene_pool` variant
    /// (1.E.3c) will iterate this map and bind a `GpuGlobalMeshPool`.
    gpu_meshes: HashMap<Handle<MeshletMesh>, GpuMeshletMesh>,
    /// Single-mesh path's "active" handle — populated by
    /// [`Self::ensure_gpu_mesh`] (first call) so [`Self::render_with_assets`]
    /// can look up the gpu mesh without an explicit argument.
    active_handle: Option<Handle<MeshletMesh>>,

    meshlet_bgl: wgpu::BindGroupLayout,

    vbuf_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    color_view: wgpu::TextureView,

    vbuf_texture: wgpu::Texture,
    depth_texture: wgpu::Texture,
    color_texture: wgpu::Texture,

    size: (u32, u32),
    instance_capacity: u32,
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
            gpu_meshes: HashMap::new(),
            active_handle: None,
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

    /// Ensures `mesh` is uploaded to GPU under `handle`. Idempotent —
    /// repeat calls with the same handle skip re-upload. Also registers
    /// the CPU-side asset with the [`MeshletPipeline`] so the ECS query
    /// can pick the entity up.
    ///
    /// The first ensured handle becomes the "active" mesh used by
    /// [`Self::render_with_assets`]. Multi-mesh scenes (1.E.3c) will
    /// extend the lookup to `MeshInstance::mesh_id`.
    pub fn ensure_gpu_mesh(
        &mut self,
        device: &wgpu::Device,
        handle: Handle<MeshletMesh>,
        mesh: &MeshletMesh,
    ) {
        self.pipeline.register_mesh(handle, mesh);
        if !self.gpu_meshes.contains_key(&handle) {
            self.gpu_meshes.insert(handle, mesh.upload(device));
        }
        if self.active_handle.is_none() {
            self.active_handle = Some(handle);
        }
    }

    /// Number of `Handle<MeshletMesh>` entries currently uploaded to GPU.
    pub fn gpu_mesh_count(&self) -> u32 {
        self.gpu_meshes.len() as u32
    }

    pub fn active_handle(&self) -> Option<Handle<MeshletMesh>> {
        self.active_handle
    }

    /// Walks `Assets<MeshletMesh>` and uploads any meshlet mesh
    /// referenced by a visible `MeshRenderer` that is not yet cached.
    /// Bridges the gap between asset loading (CPU) and the GPU pool
    /// without forcing the caller to hand-register each handle.
    pub fn sync_assets_to_gpu(&mut self, device: &wgpu::Device, resources: &Resources) {
        let Some(assets) = resources.get::<Assets<MeshletMesh>>() else {
            return;
        };
        let visible = self.pipeline.collect_referenced_handles(resources);
        for handle in visible {
            if self.gpu_meshes.contains_key(&handle) {
                continue;
            }
            if let Some(mesh) = assets.get(handle) {
                let gpu = mesh.upload(device);
                self.pipeline.register_mesh(handle, mesh);
                self.gpu_meshes.insert(handle, gpu);
                if self.active_handle.is_none() {
                    self.active_handle = Some(handle);
                }
            }
        }
    }

    /// Convenience wrapper that pulls the active gpu mesh from the
    /// internal cache, then forwards to [`Self::render`]. Returns
    /// [`MeshletRenderStats::default`] if no mesh is active or no
    /// instances were collected.
    pub fn render_with_assets(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &Resources,
        view_proj: Mat4,
        cam_pos: Vec3,
    ) -> MeshletRenderStats {
        let Some(handle) = self.active_handle else {
            return MeshletRenderStats::default();
        };
        let Some(gpu_mesh) = self.gpu_meshes.get(&handle) else {
            return MeshletRenderStats::default();
        };
        self.render(device, queue, resources, gpu_mesh, view_proj, cam_pos)
    }

    /// Records + submits one frame of the meshlet pipeline driven by
    /// `resources`'s ECS query.
    ///
    /// Returns [`MeshletRenderStats`] so callers (tests, HUD) can
    /// observe what the stage actually drew. Returns zero stats when
    /// the ECS query yields no instances — the stage neither uploads
    /// nor clears in that case; the previous frame's color stays in
    /// place.
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &Resources,
        gpu_mesh: &GpuMeshletMesh,
        view_proj: Mat4,
        cam_pos: Vec3,
    ) -> MeshletRenderStats {
        let instances = self.pipeline.collect_scene_instances(resources);
        if instances.is_empty() {
            return MeshletRenderStats::default();
        }
        assert!(
            (instances.len() as u32) <= self.instance_capacity,
            "MeshletRenderStage: collected {} instances exceeds capacity {}",
            instances.len(),
            self.instance_capacity,
        );

        self.scene.upload_instances(queue, &instances);
        let meshlets_per_mesh = gpu_mesh.meshlet_count;
        let cull_params = CullParams::new(view_proj, cam_pos, meshlets_per_mesh);
        let scene_params =
            SceneCullParams::new(instances.len() as u32, meshlets_per_mesh);

        let meshlet_bg = meshlet_bind_group(device, &self.meshlet_bgl, gpu_mesh);
        let material_bg = self.material_pool.bind_group(device);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("meshlet_render_stage_encoder"),
        });

        self.cull.dispatch_scene(
            device,
            queue,
            &mut encoder,
            gpu_mesh,
            &self.scene,
            &cull_params,
            &scene_params,
        );
        self.rasterizer.render_scene(
            device,
            queue,
            &mut encoder,
            &self.vbuf_view,
            &self.depth_view,
            &meshlet_bg,
            &self.cull,
            &self.scene,
            view_proj,
            0,
        );
        self.deferred.shade_scene(
            device,
            queue,
            &mut encoder,
            &self.vbuf_view,
            &self.color_view,
            &meshlet_bg,
            &material_bg,
            &self.cull,
            &self.scene,
            view_proj,
            self.size,
        );

        queue.submit(std::iter::once(encoder.finish()));

        MeshletRenderStats {
            instances_uploaded: instances.len() as u32,
            cull_threads: scene_params.instance_count * scene_params.meshlets_per_mesh,
        }
    }
}

fn create_2d_attachment(
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
