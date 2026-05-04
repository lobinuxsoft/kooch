//! Per-frame work for [`MeshletRenderStage`]: resize, GPU mesh cache
//! upkeep, ECS asset sync, and the cull → vbuf → deferred chain.
//! Lives in its own file so the parent stage module stays under the
//! 400-LoC ceiling and the "construct vs run" split is visible at a
//! glance.

use glam::{Mat4, Vec3};

use ome_core::Guid;
use ome_core::asset_loader::AssetServer;
use ome_core::assets::Assets;
use ome_core::resource::Resources;

use crate::meshlet::asset::MeshletMesh;
use crate::meshlet::cull::CullParams;
use crate::meshlet::deferred::DEFERRED_COLOR_FORMAT;
use crate::meshlet::gpu_meshlet::{meshlet_bind_group, GpuMeshletMesh};
use crate::meshlet::scene::SceneCullParams;
use crate::meshlet::vis_buffer::VISIBILITY_BUFFER_FORMAT;

use super::{create_2d_attachment, MeshletRenderStage, MeshletRenderStats};

impl MeshletRenderStage {
    /// Recreates the stage's vbuf / depth / color textures at
    /// `new_size` if it differs from the current size. The cull,
    /// rasterizer, deferred and material pool are unaffected — only
    /// the per-pixel attachments need to grow with the destination
    /// surface.
    pub fn resize(&mut self, device: &wgpu::Device, new_size: (u32, u32)) {
        if new_size == self.size {
            return;
        }
        assert!(
            new_size.0 > 0 && new_size.1 > 0,
            "MeshletRenderStage::resize requires non-zero dimensions"
        );

        let (vbuf_texture, vbuf_view) = create_2d_attachment(
            device,
            "meshlet_render_stage_vbuf",
            new_size,
            VISIBILITY_BUFFER_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let (depth_texture, depth_view) = create_2d_attachment(
            device,
            "meshlet_render_stage_depth",
            new_size,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let (color_texture, color_view) = create_2d_attachment(
            device,
            "meshlet_render_stage_color",
            new_size,
            DEFERRED_COLOR_FORMAT,
            wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );

        self.vbuf_texture = vbuf_texture;
        self.vbuf_view = vbuf_view;
        self.depth_texture = depth_texture;
        self.depth_view = depth_view;
        self.color_texture = color_texture;
        self.color_view = color_view;
        self.size = new_size;
    }

    /// Ensures `mesh` is uploaded to GPU under `guid`. Idempotent —
    /// repeat calls with the same GUID skip re-upload. Also registers
    /// the CPU-side asset with the [`MeshletPipeline`](super::MeshletPipeline)
    /// so the ECS query can pick the entity up.
    ///
    /// The first ensured GUID becomes the "active" mesh used by
    /// [`Self::render_with_assets`]. Multi-mesh scenes (1.E.3c) will
    /// extend the lookup to `MeshInstance::mesh_id`.
    pub fn ensure_gpu_mesh(
        &mut self,
        device: &wgpu::Device,
        guid: Guid,
        mesh: &MeshletMesh,
    ) {
        self.pipeline.register_mesh(guid, mesh);
        if !self.gpu_meshes.contains_key(&guid) {
            self.gpu_meshes.insert(guid, mesh.upload(device));
        }
        if self.active_guid.is_none() {
            self.active_guid = Some(guid);
        }
    }

    /// Number of distinct meshlet meshes currently uploaded to GPU
    /// (one per registered GUID).
    pub fn gpu_mesh_count(&self) -> u32 {
        self.gpu_meshes.len() as u32
    }

    pub fn active_guid(&self) -> Option<Guid> {
        self.active_guid
    }

    /// Resolves every visible `MeshRenderer.mesh` GUID through the
    /// `AssetServer`, fetches the meshlet asset from
    /// `Assets<MeshletMesh>`, and uploads any GUID that is not yet
    /// GPU-resident.
    ///
    /// Idempotent: GUIDs already present in `gpu_meshes` are skipped
    /// without touching the AssetServer or Assets storage. Per-frame
    /// cost when steady-state is one ECS query + N hashmap lookups.
    ///
    /// Failure modes (logged, never panic):
    /// - `AssetServer` resource missing → noop, log warn.
    /// - GUID not registered in `AssetDatabase` → log warn, skip entity.
    /// - Loader rejects the bytes → log warn, skip entity.
    /// - `Assets<MeshletMesh>` missing or stale handle → log warn, skip.
    pub fn sync_assets_to_gpu(&mut self, device: &wgpu::Device, resources: &mut Resources) {
        let pending: Vec<Guid> = self
            .pipeline
            .collect_referenced_guids(resources)
            .into_iter()
            .filter(|guid| !self.gpu_meshes.contains_key(guid))
            .collect();
        if pending.is_empty() {
            return;
        }

        for guid in pending {
            // Take the AssetServer out so we can pass `resources`
            // (which holds `Assets<MeshletMesh>`) by &mut into the
            // load call. Re-insert before any continue/return so we
            // never leak the resource.
            let Some(mut server) = resources.remove::<AssetServer>() else {
                tracing::warn!(
                    target: "ome_render::meshlet::sync",
                    "AssetServer resource missing; skipping meshlet asset sync",
                );
                return;
            };
            let load_result = server.load_by_guid::<MeshletMesh>(guid, resources);
            resources.insert(server);

            let handle = match load_result {
                Ok(h) => h,
                Err(e) => {
                    tracing::warn!(
                        target: "ome_render::meshlet::sync",
                        guid = %guid,
                        error = %e,
                        "failed to load meshlet asset by GUID",
                    );
                    continue;
                }
            };

            let Some(assets) = resources.get::<Assets<MeshletMesh>>() else {
                tracing::warn!(
                    target: "ome_render::meshlet::sync",
                    "Assets<MeshletMesh> resource missing after load; aborting sync",
                );
                return;
            };
            let Some(mesh) = assets.get(handle) else {
                tracing::warn!(
                    target: "ome_render::meshlet::sync",
                    guid = %guid,
                    "loaded handle resolved to empty Assets<MeshletMesh> entry",
                );
                continue;
            };

            self.ensure_gpu_mesh(device, guid, mesh);
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
        let Some(guid) = self.active_guid else {
            return MeshletRenderStats::default();
        };
        let Some(gpu_mesh) = self.gpu_meshes.get(&guid) else {
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
