//! Per-frame meshlet culling dispatcher.
//!
//! Split in two along one question — *does its content depend on where
//! the camera is?*
//!
//! - [`MeshletCullPipelines`] — compute pipelines and bind group
//!   layouts. **No**, so one instance serves every view.
//! - [`MeshletCull`] — the per-frame [`CullParams`] / [`HiZTestParams`]
//!   UBOs, the visible-meshlet output buffer and the atomic counter
//!   that doubles as the indirect-draw `instance_count` source.
//!   **Yes**, so each view owns a set.
//!
//! One [`MeshletCull`] is reused across frames *for its view*;
//! [`MeshletCull::dispatch`] or [`MeshletCull::dispatch_with_hi_z`] is
//! called once per frame per view inside the render encoder, after that
//! view's camera matrices are known, taking the shared pipelines as its
//! first argument.
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
mod pipelines;
mod types;

pub use pipelines::MeshletCullPipelines;
pub use types::{DrawIndirectArgs, HiZTestParams};

/// One **view's** cull state: every buffer the cull pass writes.
///
/// Split from [`MeshletCullPipelines`] for #592. The test applied to
/// each field was "does its content depend on where the camera is?" —
/// everything here answered yes, so a second view needs its own set.
/// Sharing them is the shape of [bevyengine/bevy#15182]: two viewports
/// overlapping, each overwriting the other's survivor list mid-frame.
///
/// The output buffers (`visible_*`, `indirect_args`) are sized at
/// construction; call [`Self::ensure_capacity`] when the scene grows.
///
/// [bevyengine/bevy#15182]: https://github.com/bevyengine/bevy/issues/15182
pub struct MeshletCull {
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
    /// Per-thread reject-reason tag buffer (#454.4). One u32 per
    /// cull thread; `capacity` slots so it grows in lock-step with
    /// `visible_meshlets`. Cleared each frame by the dispatcher
    /// before the cull pass and read by the reject-overlay raster
    /// pass to drive the rejection bounding-box visualization.
    pub(super) reject_reasons: wgpu::Buffer,
    /// Per-stage cull survivor counters (#454.6). 16-byte buffer
    /// holding `atomic<u32>; 4`:
    ///   [0] = after_frustum   (passed frustum test)
    ///   [1] = after_backface  (passed frustum + backface)
    ///   [2] = after_hi_z      (passed all three; only Hi-Z 2-pass
    ///         path writes — non-Hi-Z atomic path leaves it 0)
    ///   [3] = total_visible   (terminal — equals visible_count)
    /// AtomicAdded at each stage tail when
    /// `CullParams.debug_active != 0`. Cleared per frame; readback
    /// drives the editor's stats overlay.
    pub(super) stage_counters: wgpu::Buffer,

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
            target: "kooch_render::meshlet::cull",
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
        // Reject-reasons mirrors visible_meshlets: one u32 slot per
        // cull thread, sized to the same `capacity`. Growing them in
        // lock-step keeps a single `ensure_capacity` call sufficient
        // for both the production rasterizer and the debug overlay.
        let reject_reasons = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_reject_reasons"),
            size: new_capacity as u64 * 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        tracing::info!(
            target: "kooch_render::meshlet::cull",
            old_capacity = self.capacity,
            new_capacity,
            required,
            "grew visible_meshlets + reject_reasons buffers to fit scene",
        );
        self.visible_meshlets = visible_meshlets;
        self.reject_reasons = reject_reasons;
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

    /// Per-thread reject-reason tag buffer (#454.4). One u32 per
    /// cull thread; the cull pass writes a reason code on every
    /// return path when `CullParams.debug_active != 0`. The reject
    /// overlay raster pass reads this back to paint rejection
    /// bounding boxes on top of the shaded image.
    pub fn reject_reasons_buffer(&self) -> &wgpu::Buffer {
        &self.reject_reasons
    }

    /// Per-stage cull survivor counters (#454.6). 16-byte buffer
    /// holding 4 atomic u32s — see the field doc on
    /// [`Self::stage_counters`] for the slot layout. Read back per
    /// frame by [`MeshletStageCounters`](super::stage_counters::MeshletStageCounters)
    /// when the editor's debug mode is one that gates `debug_active`
    /// on (any reject-overlay variant today).
    pub fn stage_counters_buffer(&self) -> &wgpu::Buffer {
        &self.stage_counters
    }

    /// `wgpu::Buffer` holding the per-frame `SceneCullParams` UBO.
    /// Re-exported so the reject-overlay pass can bind the same
    /// `(instance_count, meshlets_per_mesh)` the cull pass dispatched
    /// against — using a different value here would either over- or
    /// under-iterate the reject_reasons array.
    pub fn scene_params_buffer(&self) -> &wgpu::Buffer {
        &self.scene_params_buffer
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
        const CULL_SHADER_SOURCE: &str = concat!(
            include_str!("../../../shaders/meshlet_cull/common.wgsl"),
            include_str!("../../../shaders/meshlet_cull/basic.wgsl"),
            include_str!("../../../shaders/meshlet_cull/scene.wgsl"),
            include_str!("../../../shaders/meshlet_cull/pool.wgsl"),
            include_str!("../../../shaders/meshlet_cull/atomic.wgsl"),
            include_str!("../../../shaders/meshlet_cull/atomic_hi_z.wgsl"),
        );
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
