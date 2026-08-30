//! Cull pipelines and bind group layouts — shared by every view.
//!
//! Split out of [`super::MeshletCull`] for #592. The division is the
//! question "does its content depend on where the camera is?":
//!
//! - **No** — compute pipelines and bind group layouts. They describe
//!   *how* to cull, which is identical for the game view, the editor
//!   viewport, a shadow cascade and a render-to-texture camera. One
//!   instance is shared by all of them.
//! - **Yes** — every buffer the cull writes. Those live in
//!   [`super::MeshletCull`], one per view.
//!
//! Duplicating this struct per view would recompile nine compute
//! pipelines for each camera. Virtual Shadow Maps request one view per
//! page per light, so "one renderer per camera" is not an option that
//! survives contact with the roadmap.

/// Compute pipelines + bind group layouts for meshlet culling.
///
/// Constructed once per [`crate::meshlet::MeshletRenderStage`] and
/// passed by reference to every `MeshletCull::dispatch*` call.
pub struct MeshletCullPipelines {
    pub(super) pipeline: wgpu::ComputePipeline,
    pub(super) pipeline_hi_z: wgpu::ComputePipeline,
    pub(super) pipeline_scene: wgpu::ComputePipeline,
    pub(super) pipeline_scene_pool: wgpu::ComputePipeline,
    pub(super) pipeline_lod_compute_group_max_err: wgpu::ComputePipeline,
    pub(super) pipeline_cull_scene_pool_atomic: wgpu::ComputePipeline,
    /// Pass-1 (`cs_lod_compute_group_max_err`) recompiled against the
    /// extended cull layout the Hi-Z 2-pass entry uses (#445). Same
    /// shader entry point — only the pipeline_layout changes so
    /// `culled_meshlets` / `culled_count` and `hi_z_*` slots are
    /// declared even though pass 1 doesn't touch them.
    pub(super) pipeline_lod_compute_group_max_err_hi_z: wgpu::ComputePipeline,
    /// Pass A of the 2-pass Hi-Z cull (#445). Mirror of
    /// `cs_cull_scene_pool_atomic` plus a Hi-Z occlusion test against
    /// the previous frame's pyramid; rejects land in `culled_meshlets`
    /// for pass B to retest.
    pub(super) pipeline_cull_scene_pool_atomic_hi_z: wgpu::ComputePipeline,
    /// Pass B (#445). Drains `culled_meshlets[0..culled_count]`,
    /// re-tests each entry against this frame's freshly-built
    /// pyramid, and appends survivors to `visible_meshlets`. Same
    /// pipeline_layout as pass A so the orchestrator reuses the
    /// extended cull / scene-with-hi-z bind groups (with the pyramid
    /// view swapped from `hiz_prev` to `hiz_curr`).
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
    /// Cull BGL used by the Hi-Z 2-pass path. Identical to `cull_bgl`
    /// for bindings 0-3 plus two read_write storage slots at 4-5 for
    /// `culled_meshlets` + `culled_count`. Existing entry points keep
    /// using `cull_bgl` so their dispatches stay binary-compatible.
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
    /// Single-binding BGL for the per-thread `reject_reasons` buffer
    /// (#454.4). Bound at group(4) of the scene-pool atomic cull
    /// pipeline layout; the reject-overlay raster pass reuses it
    /// to read the same buffer back at draw time.
    pub(super) debug_bgl: wgpu::BindGroupLayout,
    /// Group 3 of the chunked cull: `group_max_err`, the per-mesh
    /// bounding spheres and the chunk list.
    pub(super) chunked_bgl: wgpu::BindGroupLayout,
}

impl MeshletCullPipelines {
    /// Bind group layout for the per-thread `reject_reasons` buffer
    /// (group(4) of the scene-pool atomic cull pipeline). Re-exported
    /// so the reject-overlay raster pass can build a bind group
    /// against the same handle the cull pipeline writes through.
    pub fn debug_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.debug_bgl
    }

    /// Bind group layout for the Hi-Z 2-pass entry's group(0) — the
    /// 4-binding `cull_bgl` plus `culled_meshlets` (4) and
    /// `culled_count` (5).
    pub fn extended_cull_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.extended_cull_bgl
    }

    /// Bind group layout for the Hi-Z 2-pass entry's group(2) — the
    /// 2-binding `scene_bgl` plus the `HiZParams` UBO at 2 and the
    /// pyramid texture at 3.
    pub fn scene_with_hi_z_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.scene_with_hi_z_bgl
    }

    /// Bind group layout describing the cull shader's group(0).
    /// Re-exported so future passes can extend it.
    pub fn cull_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.cull_bgl
    }

    /// Bind group layout describing the meshlet pool's group(1) — the
    /// rasterizer reuses the exact same handle so the cull and draw
    /// passes agree on storage-buffer slot numbering.
    pub fn meshlet_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.meshlet_bgl
    }

    /// Bind group layout for the Hi-Z test (group 1 of `cs_cull_hi_z`).
    pub fn hi_z_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.hi_z_bgl
    }

    /// Bind group layout for the scene-wide cull (group 2 of
    /// `cs_cull_scene`): instance storage + `SceneCullParams` UBO.
    pub fn scene_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.scene_bgl
    }

    /// Bind group layout for the cull-only subset of the multi-mesh
    /// pool (group 1 of `cs_cull_scene_pool`): mesh_descriptors at
    /// binding 0 + meshlets at binding 1. The cull pass omits the
    /// vertex / meshlet_vertex / meshlet_triangle bindings exposed
    /// by [`crate::meshlet::pool::GpuGlobalMeshPool::bind_group_layout`]
    /// to stay under the wgpu compute-stage storage-buffer limit;
    /// the rasterizer + deferred shaders use the full layout.
    pub fn pool_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.pool_bgl
    }

    /// Bind group layout for the 2-pass cull's per-group err buffer
    /// (group 3 of `cs_lod_compute_group_max_err` /
    /// `cs_cull_scene_pool_atomic`).
    pub fn group_err_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.group_err_bgl
    }
}
