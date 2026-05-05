//! Per-frame meshlet culling dispatcher.
//!
//! Owns the compute pipelines, per-frame [`CullParams`] / [`HiZTestParams`]
//! UBOs, the visible-meshlet output buffer, and the atomic counter
//! that doubles as the indirect-draw `instance_count` source. One
//! [`MeshletCull`] is shared across frames; [`MeshletCull::dispatch`]
//! or [`MeshletCull::dispatch_with_hi_z`] is called once per frame
//! inside the render encoder, after camera matrices are known.
//!
//! # Pipeline (frustum + cone variant)
//!
//! ```text
//! camera matrices  →  CullParams UBO          (CPU upload)
//!                          │
//!                          ▼
//!     reset(visible_count = 0)                (clear pass)
//!                          │
//!                          ▼
//!     dispatch cs_cull, ⌈meshlet_count/64⌉ workgroups
//!                          │
//!                          ▼
//!         visible_meshlets[0..visible_count]   (atomic-appended)
//!                          │
//!                          ▼
//!     copy_buffer_to_buffer(visible_count → indirect_args[+4])
//!                          │
//!                          ▼
//!  draw_indirect(indirect_args)
//! ```
//!
//! The `instance_count` slot (offset 4 inside `DrawIndirectArgs`) is
//! kept in lock-step with `visible_count` via a single-shot
//! buffer-to-buffer copy so the cull shader stays free of indirect-args
//! bookkeeping. `vertex_count` (offset 0) is set once at construction
//! and never changes.

mod dispatch;
mod init;
mod types;

pub use types::{DrawIndirectArgs, HiZTestParams};

/// Owns one frame's worth of cull state. The output buffers (`visible_*`,
/// `indirect_args`) are sized at construction; recreate the dispatcher if
/// scene meshlet count grows past `capacity`.
pub struct MeshletCull {
    pub(super) pipeline: wgpu::ComputePipeline,
    pub(super) pipeline_hi_z: wgpu::ComputePipeline,
    pub(super) pipeline_scene: wgpu::ComputePipeline,
    pub(super) pipeline_scene_pool: wgpu::ComputePipeline,
    pub(super) cull_bgl: wgpu::BindGroupLayout,
    pub(super) hi_z_bgl: wgpu::BindGroupLayout,
    pub(super) scene_bgl: wgpu::BindGroupLayout,
    pub(super) meshlet_bgl: wgpu::BindGroupLayout,
    pub(super) pool_bgl: wgpu::BindGroupLayout,

    pub(super) params_buffer: wgpu::Buffer,
    pub(super) hi_z_params_buffer: wgpu::Buffer,
    pub(super) scene_params_buffer: wgpu::Buffer,
    pub(super) visible_meshlets: wgpu::Buffer,
    pub(super) visible_count: wgpu::Buffer,
    pub(super) indirect_args: wgpu::Buffer,

    pub(super) capacity: u32,
    pub(super) vertex_count_per_instance: u32,
}

impl MeshletCull {
    /// Storage capacity (in meshlets) of the visible-output buffer.
    /// Dispatching against a `GpuMeshletMesh` with more meshlets than
    /// this is a programmer error — the cull shader bounds-checks
    /// `meshlet_count`, so the excess are simply ignored.
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Number of vertices the rasterizer fetches per meshlet instance.
    /// Equals `MAX_TRIANGLES * 3`; degenerate triangles (idx >=
    /// triangle_count) collapse to off-screen vertices in the meshlet
    /// vertex shader.
    pub fn vertex_count_per_instance(&self) -> u32 {
        self.vertex_count_per_instance
    }

    /// `wgpu::Buffer` holding `[DrawIndirectArgs; 1]`. Bound as
    /// `BufferUsages::INDIRECT | STORAGE` so future variants can also
    /// write it from the cull shader.
    pub fn indirect_args_buffer(&self) -> &wgpu::Buffer {
        &self.indirect_args
    }

    /// `wgpu::Buffer` holding `array<u32>` of meshlet ids that survived
    /// culling. Length is `visible_count` (read from the atomic). The
    /// rasterizer binds this and indexes by `@builtin(instance_index)`.
    pub fn visible_meshlets_buffer(&self) -> &wgpu::Buffer {
        &self.visible_meshlets
    }

    /// `wgpu::Buffer` holding `atomic<u32>` (single u32). Written by the
    /// cull shader, read back by tests, and copied into the indirect
    /// args' `instance_count` slot.
    pub fn visible_count_buffer(&self) -> &wgpu::Buffer {
        &self.visible_count
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_indirect_args_layout_is_pod() {
        // Must match wgpu::DrawIndirectArgs exactly so we can write
        // straight into an INDIRECT-usage buffer.
        assert_eq!(std::mem::size_of::<DrawIndirectArgs>(), 16);
    }

    #[test]
    fn draw_indirect_args_default_is_zero() {
        let args = DrawIndirectArgs::default();
        assert_eq!(args.vertex_count, 0);
        assert_eq!(args.instance_count, 0);
        assert_eq!(args.first_vertex, 0);
        assert_eq!(args.first_instance, 0);
    }

    #[test]
    fn hi_z_test_params_layout() {
        // 64-byte mat4 + 8-byte vec2 + 4-byte u32 + 4-byte pad = 80 B.
        assert_eq!(std::mem::size_of::<HiZTestParams>(), 80);
    }

    #[test]
    fn cull_shader_parses_and_validates() {
        const CULL_SHADER_SOURCE: &str =
            include_str!("../../../shaders/meshlet_cull.wgsl");
        let module = naga::front::wgsl::parse_str(CULL_SHADER_SOURCE)
            .expect("meshlet_cull.wgsl should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("meshlet_cull.wgsl should validate");
    }
}
