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

    pub(super) params_buffer: wgpu::Buffer,
    pub(super) hi_z_params_buffer: wgpu::Buffer,
    pub(super) scene_params_buffer: wgpu::Buffer,
    pub(super) visible_meshlets: wgpu::Buffer,
    pub(super) visible_count: wgpu::Buffer,
    /// Pass-A reject queue for the Hi-Z 2-pass cull (#445). Sized to
    /// the same capacity as `visible_meshlets` so the worst case where
    /// every meshlet is occluded fits without overflow. Cleared each
    /// frame before pass A.
    pub(super) culled_meshlets: wgpu::Buffer,
    /// Atomic counter for `culled_meshlets`. Pass B reads this both
    /// as the workgroup count and the loop bound.
    pub(super) culled_count: wgpu::Buffer,
    pub(super) indirect_args: wgpu::Buffer,
    /// Per-group atomic<u32> buffer the 2-pass cull (#465) writes
    /// in pass 1 and reads in pass 2. Sized to `group_capacity`,
    /// resized geometrically by [`Self::ensure_group_capacity`] when
    /// a scene's pool grows past it. Cleared each frame before pass 1.
    pub(super) group_max_err: wgpu::Buffer,
    pub(super) group_capacity: u32,

    pub(super) capacity: u32,
    pub(super) vertex_count_per_instance: u32,
}

impl MeshletCull {
    /// Storage capacity (in meshlets) of the visible-output buffer.
    /// Use [`Self::ensure_capacity`] before dispatching to grow the
    /// buffer when a scene exceeds the current allocation.
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Storage capacity (in u32 slots) of the per-group `group_max_err`
    /// buffer. Use [`Self::ensure_group_capacity`] before dispatching
    /// the 2-pass atomic cull when a scene's pool exceeds the current
    /// allocation.
    pub fn group_capacity(&self) -> u32 {
        self.group_capacity
    }

    /// Grows `group_max_err` so it covers at least `required` group
    /// ids. No-op when current capacity already covers the request.
    /// Geometric growth — same pattern as [`Self::ensure_capacity`].
    pub fn ensure_group_capacity(&mut self, device: &wgpu::Device, required: u32) {
        if required <= self.group_capacity {
            return;
        }
        let new_capacity = required
            .checked_next_power_of_two()
            .unwrap_or(required)
            .max(self.group_capacity.saturating_mul(2));
        self.group_max_err = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_group_max_err"),
            size: new_capacity as u64 * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        tracing::info!(
            target: "ome_render::meshlet::cull",
            old_capacity = self.group_capacity,
            new_capacity,
            required,
            "grew group_max_err buffer to fit scene",
        );
        self.group_capacity = new_capacity;
    }

    /// Grows `visible_meshlets` so it can hold at least `required`
    /// surviving meshlets. No-op when current capacity already
    /// covers the request. Growth is geometric (doubles) and rounds
    /// up to the next power of two to absorb subsequent jumps without
    /// reallocating every frame.
    ///
    /// The replaced buffer is dropped at the end of this frame's
    /// command submission — wgpu's resource lifetime tracking keeps
    /// it alive until in-flight command buffers no longer reference
    /// it.
    pub fn ensure_capacity(&mut self, device: &wgpu::Device, required: u32) {
        if required <= self.capacity {
            return;
        }
        let new_capacity = required
            .checked_next_power_of_two()
            .unwrap_or(required)
            .max(self.capacity.saturating_mul(2));
        let visible_meshlets = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_visible_ids"),
            size: new_capacity as u64 * 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        tracing::info!(
            target: "ome_render::meshlet::cull",
            old_capacity = self.capacity,
            new_capacity,
            required,
            "grew visible_meshlets buffer to fit scene",
        );
        self.visible_meshlets = visible_meshlets;
        self.capacity = new_capacity;
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

    /// Pass-A reject queue for the Hi-Z 2-pass cull (#445). Each
    /// element is a `(instance_id << 16) | global_meshlet_idx` packed
    /// just like `visible_meshlets`. Pass B re-tests every entry up
    /// to `culled_count` against the freshly-built pyramid.
    pub fn culled_meshlets_buffer(&self) -> &wgpu::Buffer {
        &self.culled_meshlets
    }

    /// Atomic counter for `culled_meshlets`. Doubles as the pass-B
    /// dispatch length.
    pub fn culled_count_buffer(&self) -> &wgpu::Buffer {
        &self.culled_count
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
