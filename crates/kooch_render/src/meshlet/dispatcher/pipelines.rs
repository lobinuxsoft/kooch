//! Cull pipelines and bind group layouts — shared by every view.

/// Compute pipelines + bind group layouts for meshlet culling.
pub struct MeshletCullPipelines {
    pub(super) pipeline: wgpu::ComputePipeline,
    pub(super) pipeline_hi_z: wgpu::ComputePipeline,
    pub(super) pipeline_scene: wgpu::ComputePipeline,
    pub(super) pipeline_scene_pool: wgpu::ComputePipeline,
    pub(super) pipeline_lod_compute_group_max_err: wgpu::ComputePipeline,
    pub(super) pipeline_cull_scene_pool_atomic: wgpu::ComputePipeline,
    /// Pass-1 (`cs_lod_compute_group_max_err`) recompiled against the extended cull layout the Hi-Z
    /// 2-pass entry uses (#445).
    pub(super) pipeline_lod_compute_group_max_err_hi_z: wgpu::ComputePipeline,
    /// Pass A of the 2-pass Hi-Z cull (#445). Mirror of `cs_cull_scene_pool_atomic` plus a Hi-Z
    /// occlusion test against the previous frame's pyramid; rejects land in `culled_meshlets` for
    /// pass B to retest.
    pub(super) pipeline_cull_scene_pool_atomic_hi_z: wgpu::ComputePipeline,
    /// Pass B (#445). Drains `culled_meshlets[0..culled_count]`, re-tests each entry against this
    /// frame's freshly-built pyramid, and appends survivors to `visible_meshlets`.
    pub(super) pipeline_cull_pass_b: wgpu::ComputePipeline,
    /// Level one of the two-level cull (#1002): one thread per
    /// INSTANCE, writing a chunk per 64 meshlets of each survivor.
    pub(super) pipeline_cull_instances: wgpu::ComputePipeline,
    /// Turns the chunk count — a number that exists only on the GPU —
    /// into the indirect args `pipeline_cull_expand` runs under.
    pub(super) pipeline_cull_expand_args: wgpu::ComputePipeline,
    /// #465's pass 1, reached from a chunk instead of a rectangle.
    pub(super) pipeline_lod_group_max_err_chunked: wgpu::ComputePipeline,
    /// #465's pass 2, reached from a chunk. The meshlet domain is
    /// entered at each instance's OWN count instead of at the scene's
    /// heaviest mesh — which is the whole of #1002.
    pub(super) pipeline_cull_scene_pool_atomic_chunked: wgpu::ComputePipeline,

    pub(super) cull_bgl: wgpu::BindGroupLayout,
    /// Cull BGL used by the Hi-Z 2-pass path. Identical to `cull_bgl` for bindings 0-3 plus two
    /// read_write storage slots at 4-5 for `culled_meshlets` + `culled_count`. Existing entry
    /// points keep using `cull_bgl` so their dispatches stay binary-compatible.
    pub(super) extended_cull_bgl: wgpu::BindGroupLayout,
    pub(super) hi_z_bgl: wgpu::BindGroupLayout,
    pub(super) scene_bgl: wgpu::BindGroupLayout,
    /// Scene BGL used by the Hi-Z 2-pass path. Identical to `scene_bgl`
    /// for bindings 0-1 plus a uniform `HiZParams` at 2 and the multi-
    /// mip pyramid texture at 3.
    pub(super) scene_with_hi_z_bgl: wgpu::BindGroupLayout,
    pub(super) meshlet_bgl: wgpu::BindGroupLayout,
    pub(super) pool_bgl: wgpu::BindGroupLayout,
    pub(super) group_err_bgl: wgpu::BindGroupLayout,
    /// Single-binding BGL for the per-thread `reject_reasons` buffer (#454.4). Bound at group(4) of
    /// the scene-pool atomic cull pipeline layout; the reject-overlay raster pass reuses it to read
    /// the same buffer back at draw time.
    pub(super) debug_bgl: wgpu::BindGroupLayout,
    /// Group 3 of the chunked cull: `group_max_err`, the per-mesh
    /// bounding spheres and the chunk list.
    pub(super) chunked_bgl: wgpu::BindGroupLayout,
}

impl MeshletCullPipelines {
    /// Bind group layout for the per-thread `reject_reasons` buffer (group(4) of the scene-pool
    /// atomic cull pipeline). Re-exported so the reject-overlay raster pass can build a bind group
    /// against the same handle the cull pipeline writes through.
    pub fn debug_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.debug_bgl
    }

    /// Bind group layout describing the meshlet pool's group(1) — the
    /// rasterizer reuses the exact same handle so the cull and draw
    /// passes agree on storage-buffer slot numbering.
    pub fn meshlet_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.meshlet_bgl
    }

    /// Bind group layout for the scene-wide cull (group 2 of
    /// `cs_cull_scene`): instance storage + `SceneCullParams` UBO.
    pub fn scene_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.scene_bgl
    }

    /// Bind group layout for the cull-only subset of the multi-mesh pool (group 1 of
    /// `cs_cull_scene_pool`): mesh_descriptors at binding 0 + meshlets at binding 1.
    pub fn pool_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.pool_bgl
    }
}
