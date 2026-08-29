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

use crate::meshlet::cull::CullParams;

/// Words before the chunk list in [`MeshletCull::chunks`]. Mirrors
/// `CHUNK_LIST` in `meshlet_cull/two_level.wgsl`.
pub(super) const CHUNK_HEADER_WORDS: u64 = 6;

/// Byte offset `cs_cull_expand_args` writes the dispatch args at,
/// inside `chunks`. Mirrors `CHUNK_ARGS`. They are copied out of here
/// into [`MeshletCull::chunk_args`] before anything dispatches off
/// them.
pub(super) const CHUNK_ARGS_OFFSET: u64 = 3 * 4;

/// Three `u32` — the x, y, z of a `dispatch_workgroups_indirect`.
pub(super) const DISPATCH_ARGS_BYTES: u64 = 12;

/// Meshlets one chunk covers — the two-level cull's workgroup size.
/// Mirrors `CULL_GROUP`.
pub const CULL_CHUNK_MESHLETS: u32 = 64;

pub use dispatch::chunks_for;
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
    /// The two-level cull's chunk list and its header (#1002). One
    /// buffer because the pipeline layout is already at seven of the
    /// eight storage buffers a compute stage may bind:
    /// `[0]` chunk count, `[1]` chunks dropped, `[2]` surviving
    /// instances, `[3..6)` the indirect args the expansion runs under,
    /// then one word per chunk.
    ///
    pub(super) chunks: wgpu::Buffer,
    /// The chunk count, copied out of `chunks` into a buffer of its own
    /// so the expansion can dispatch off it.
    ///
    /// 🔴 A COPY, and not the same allocation, because wgpu refuses a
    /// buffer that is both `STORAGE_READ_WRITE` and `INDIRECT` inside
    /// one usage scope — and `chunks` has to stay bound as storage for
    /// the expansion to read the list at all. `mirror_count_to_indirect_args`
    /// solves the identical problem for `visible_count` the identical
    /// way; this is that idiom, not a new one.
    pub(super) chunk_args: wgpu::Buffer,
    /// Chunk slots `chunks` holds, not counting the header.
    pub(super) chunk_capacity: u32,
    /// Whether anyone reads `reject_reasons`. See [`Self::set_rejects`].
    pub(super) rejects: bool,

    pub(super) capacity: u32,
    pub(super) vertex_count_per_instance: u32,

    /// Bytes between two parameter slots in `params_buffer`, rounded up
    /// to the device's `min_uniform_buffer_offset_alignment`.
    pub(super) params_stride: u64,
    /// Which slot the next dispatch writes. See [`Self::stage_params`].
    pub(super) params_cursor: std::sync::atomic::AtomicU32,
}

/// How many parameter sets `params_buffer` holds.
///
/// 🔴 This buffer is a ring and not a single struct, and that is the
/// whole of #853.
///
/// `queue.write_buffer` is **not** ordered against the encoder. Every
/// write queued while a frame is being recorded is applied at the head
/// of the submit, before a single command runs — so writing one buffer
/// twice in one frame does not give two dispatches two values, it gives
/// both of them the second one.
///
/// The point-light shadow pass dispatches one cull per light per cube
/// face, and the six culls belong to the FACE, shared by every lamp.
/// So with two casting point lights, both cubes were culled against
/// whichever lamp's frustum was written last, while each was rasterised
/// with its own matrix. Every occluder the first lamp could see and the
/// second could not vanished from the first lamp's map. One lamp was
/// always correct, which is why it took a scene with two.
///
/// # 🔴 Sizing it, and the arithmetic that was wrong
///
/// The bound is how many times one cull object is dispatched inside one
/// encoder. That was read as
/// [`MAX_POINT_SHADOWS`](kooch_lighting::MAX_POINT_SHADOWS) — 32 — with
/// 64 leaving "a factor of two".
///
/// **It counted one view.** The stage renders every active view into
/// one encoder, and the editor has two. At 32 casting lamps that is
/// exactly 64 dispatches of the same cull object against 64 slots, so
/// the ring laps itself inside the frame and lamps are culled with each
/// other's frusta — #853 again, by exhaustion rather than by reuse. It
/// stayed hidden because the shipped budget was 6: twelve dispatches
/// into sixty-four never collided, and "it works" was measured on the
/// case that could not fail.
///
/// ⚠️ And the cursor is monotonic — it is never rewound at the start of
/// a frame — so a frame beginning mid-ring wraps onto its own earlier
/// slots with FEWER dispatches than there are slots. The margin has to
/// swallow that too, which is why this is generous rather than exact.
///
/// `VIEWS_ASSUMED` is the number this is allowed to be wrong about. Two
/// is what the editor uses today; four is the headroom, and
/// `the_ring_covers_the_worst_case` fails if anything makes that false.
///
/// Cross-submit reuse is not a hazard — wgpu barriers a buffer that
/// goes from uniform read to copy destination between submits.
pub(super) const VIEWS_ASSUMED: u64 = 4;

pub(super) const PARAMS_RING: u64 = kooch_lighting::MAX_POINT_SHADOWS as u64 * VIEWS_ASSUMED * 2;

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
    /// Whether anything will READ `reject_reasons` after the dispatch.
    ///
    /// 🔴 Off, the per-frame clear is skipped — and that clear is not
    /// small. The buffer is one `u32` per cull thread, so on
    /// `dense.scene` it is 67 MiB, and the virtual page raster runs
    /// seventeen culls a frame: 1.1 GiB of memset every frame for a
    /// debug overlay that is only ever wired to the CAMERA's cull.
    ///
    /// The stale values it leaves behind are exactly what the clear
    /// existed to hide, which is why this is a flag and not a deletion:
    /// whoever turns the overlay on turns this back on with it.
    pub fn set_rejects(&mut self, rejects: bool) {
        self.rejects = rejects;
    }

    /// Whether the reject buffer is worth clearing. See
    /// [`Self::set_rejects`].
    pub(super) fn reads_rejects(&self) -> bool {
        self.rejects
    }

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

    /// Grows the two-level cull's chunk list to hold `required`
    /// chunks (#1002).
    ///
    /// The worst case is `instances × ⌈heaviest / 64⌉` — the same
    /// rectangle the one-level cull dispatched, divided by the
    /// workgroup. That is a *memory* over-approximation of four bytes
    /// a chunk, where the old one was a *thread* over-approximation of
    /// nine million lanes, and only one of those is worth being
    /// precise about.
    pub fn ensure_chunk_capacity(&mut self, device: &wgpu::Device, required: u32) {
        if required <= self.chunk_capacity {
            return;
        }
        let new_capacity = required
            .checked_next_power_of_two()
            .unwrap_or(required)
            .max(self.chunk_capacity.saturating_mul(2));
        self.chunks = Self::chunk_buffer(device, new_capacity);
        tracing::info!(
            target: "kooch_render::meshlet::cull",
            old_capacity = self.chunk_capacity,
            new_capacity,
            required,
            "grew the two-level cull chunk list to fit scene",
        );
        self.chunk_capacity = new_capacity;
    }

    /// Header plus one word per chunk. See [`Self::chunks`].
    pub(super) fn chunk_buffer(device: &wgpu::Device, chunks: u32) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_cull_chunks"),
            size: (CHUNK_HEADER_WORDS + chunks as u64) * 4,
            // No `INDIRECT`: it is bound as storage for the whole pass
            // and wgpu treats the two as exclusive within one scope.
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
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
        //
        // 🔴 Unless nobody reads it, and then it is the BGL's minimum.
        // `record_reject` is already gated on `params.debug_active`, so
        // for a cull with no overlay this buffer is never written and
        // never read — and at one u32 per cull thread it was 67 MiB a
        // level, seventeen levels deep, in each of the two processes a
        // remote session runs. Measured as 9.57 GiB of VRAM held by an
        // editor sitting still (#1011).
        let reject_bytes = match self.rejects {
            true => new_capacity as u64 * 4,
            false => 4,
        };
        let reject_reasons = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_reject_reasons"),
            size: reject_bytes,
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

    /// Puts one dispatch's [`CullParams`] somewhere the previous
    /// dispatch's copy is not, and hands back the range to bind.
    ///
    /// Call once per dispatch, before building the bind group. See
    /// [`PARAMS_RING`] for why a plain write to offset 0 is wrong.
    ///
    /// `scene_params_buffer` deliberately has no ring: its contents are
    /// `(instance_count, meshlets_per_mesh)`, a property of the frame
    /// rather than of the dispatch, and every dispatch in a frame writes
    /// the same bytes. [`Self::scene_params_buffer`] is handed to the
    /// reject-overlay pass on exactly that assumption.
    pub(super) fn stage_params(
        &self,
        queue: &wgpu::Queue,
        params: &CullParams,
    ) -> wgpu::BufferBinding<'_> {
        use std::sync::atomic::Ordering;
        let slot = self.params_cursor.fetch_add(1, Ordering::Relaxed) as u64 % PARAMS_RING;
        let offset = slot * self.params_stride;
        queue.write_buffer(&self.params_buffer, offset, bytemuck::bytes_of(params));
        wgpu::BufferBinding {
            buffer: &self.params_buffer,
            offset,
            size: std::num::NonZeroU64::new(std::mem::size_of::<CullParams>() as u64),
        }
    }

    /// The slot the most recent [`Self::stage_params`] wrote, for a
    /// second pass that shares one pass's parameters — the Hi-Z
    /// 2-pass cull's pass B, which deliberately re-reads what pass A
    /// was dispatched with.
    pub(super) fn last_params(&self) -> wgpu::BufferBinding<'_> {
        use std::sync::atomic::Ordering;
        let written = self.params_cursor.load(Ordering::Relaxed).wrapping_sub(1);
        wgpu::BufferBinding {
            buffer: &self.params_buffer,
            offset: (written as u64 % PARAMS_RING) * self.params_stride,
            size: std::num::NonZeroU64::new(std::mem::size_of::<CullParams>() as u64),
        }
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
mod tests;
