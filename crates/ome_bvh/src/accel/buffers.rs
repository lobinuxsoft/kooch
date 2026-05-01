//! Pre-allocated GPU buffers backing the TLAS+BLAS pool.
//!
//! Every buffer is sized at `OmeAccel::new` and never reallocated —
//! the hot path runs `device.queue().write_buffer` slice writes only.
//! `TlasUniforms` is the lone uniform binding; everything else is
//! `STORAGE | COPY_DST`. `COPY_SRC` is enabled on the BLAS pools so
//! the regression tests can read back without parallel CPU mirrors.

use std::mem::size_of;

use crate::accel::descriptor::{ChunkDescriptor, TlasUniforms};
use crate::leaf::LeafAabb;
use crate::node::BvhNode;

/// Byte size of one TLAS scratch slot — every entry of `mortons`,
/// `sorted_indices`, `parents`, and `done` is a flat `u32`.
const TLAS_SCRATCH_ENTRY_BYTES: u64 = size_of::<u32>() as u64;

/// Per-leaf TLAS scratch size: one `u32` per live chunk, indexed by
/// the Morton-sorted leaf position `k ∈ [0, N)`. Used by `mortons`
/// and `sorted_indices`.
pub(crate) const fn tlas_per_leaf_scratch_size_bytes(max_chunks: u32) -> u64 {
    max_chunks as u64 * TLAS_SCRATCH_ENTRY_BYTES
}

/// Per-node TLAS scratch size: one `u32` per node in the flat
/// `2N - 1` Karras tree (rounded to `2N`). Karras `done[]` and
/// `parents[]` are addressed by node index — leaves at `[0, N)`,
/// internals at `[N, 2N - 1)` for this TLAS layout — so the AABB
/// propagation pass (commit 7) can flag each internal as it's
/// finalised. Same `2N` sizing as the BLAS
/// [`crate::gpu::lbvh`] `make_aux_u32_buffer`; only the slot-to-node
/// mapping differs (BLAS Karras places internals at `[0, N - 1)`,
/// leaves at `[N - 1, 2N - 1)`).
pub(crate) const fn tlas_per_node_scratch_size_bytes(max_chunks: u32) -> u64 {
    2 * max_chunks as u64 * TLAS_SCRATCH_ENTRY_BYTES
}

/// All pool buffers, owned together so the bind group can be rebuilt
/// in a single closure. Mirrors the layout the issue body pins for
/// bind group 1 bindings 5..=10, plus the TLAS GPU rebuild scratch
/// (epic #370 PR-1) which has no bind-group binding — the new TLAS
/// pipeline reads / writes it internally.
pub struct AccelBuffers {
    /// `chunk_descriptors[chunk_idx]` — `ChunkDescriptor` (64 B).
    pub chunk_descriptors: wgpu::Buffer,
    /// Concatenated BLAS nodes for every resident chunk.
    pub bvh_nodes_pool: wgpu::Buffer,
    /// Concatenated BLAS leaf metadata.
    pub leaf_aabbs_pool: wgpu::Buffer,
    /// Concatenated primitive payloads (consumer-defined `stride`).
    pub primitives_pool: wgpu::Buffer,
    /// Top-level acceleration structure nodes. Capacity `2 *
    /// max_chunks` (Karras topology max is `2N - 1`; rounded up).
    pub tlas_nodes: wgpu::Buffer,
    /// Scene-wide globals consumed by the per-role final combine.
    pub tlas_uniforms: wgpu::Buffer,

    // --- TLAS GPU rebuild scratch (epic #370 PR-1) -------------------
    // Pre-allocated to `caps.max_chunks` so the rebuild dispatch never
    // realloc'es; lifecycle is independent of the BLAS Karras scratch
    // (LbvhBuffers) — the TLAS rebuild and a BLAS rebuild can be in
    // flight on the same encoder without aliasing.
    //
    // Mortons + sorted_indices are PER-LEAF (`u32 × N`); parents +
    // done are PER-NODE (`u32 × 2N`) because Karras `done[]` /
    // `parents[]` are addressed by node index over the flat `2N - 1`
    // tree. Mirror BLAS sizing in `gpu/lbvh.rs::make_aux_u32_buffer`.
    /// Per-chunk Morton code, written by `tlas_morton.wgsl`.
    /// `u32 × max_chunks`.
    pub tlas_mortons: wgpu::Buffer,
    /// Onesweep-sorted chunk indices (input to `tlas_leaves.wgsl`).
    /// `u32 × max_chunks`.
    pub tlas_sorted_indices: wgpu::Buffer,
    /// Parent pointer per TLAS node (written by `tlas_internal.wgsl`).
    /// `u32 × 2 × max_chunks` — leaves at `[0, N)`, internals at
    /// `[N, 2N - 1)` (TLAS layout, opposite of BLAS Karras).
    pub tlas_parents: wgpu::Buffer,
    /// `done[node]` flag for the TLAS AABB propagation pass.
    /// `u32 × 2 × max_chunks` — same per-node addressing as
    /// [`AccelBuffers::tlas_parents`].
    pub tlas_done: wgpu::Buffer,
    /// Compact mapping `live_chunk_indices[k] = slot_idx` for `k ∈
    /// [0, n)`, where `n = live_chunk_count()` and `slot_idx` is the
    /// position of a live chunk inside [`AccelBuffers::chunk_descriptors`].
    /// Bridges the contiguous-live indexing the TLAS Karras passes use
    /// (`k`) to the slot-indexed `chunk_descriptors` buffer that
    /// `remove_chunk` leaves with stale entries between live chunks.
    /// Without this mapping, a `[live, evicted, live]` slot layout
    /// makes `tlas_morton.wgsl` read the evicted slot at index 1 and
    /// silently drop the second live chunk from the TLAS.
    /// `u32 × max_chunks`.
    pub tlas_live_chunk_indices: wgpu::Buffer,
}

impl AccelBuffers {
    pub fn new(
        device: &wgpu::Device,
        max_chunks: u32,
        max_nodes: u32,
        max_leaves: u32,
        max_primitives: u32,
        primitive_stride: u32,
    ) -> Self {
        let tlas_per_leaf = tlas_per_leaf_scratch_size_bytes(max_chunks);
        let tlas_per_node = tlas_per_node_scratch_size_bytes(max_chunks);
        Self {
            chunk_descriptors: make_storage(
                device,
                "ome_accel::chunk_descriptors",
                max_chunks as u64 * size_of::<ChunkDescriptor>() as u64,
                /* copy_src */ false,
            ),
            bvh_nodes_pool: make_storage(
                device,
                "ome_accel::bvh_nodes_pool",
                max_nodes as u64 * size_of::<BvhNode>() as u64,
                /* copy_src */ true,
            ),
            leaf_aabbs_pool: make_storage(
                device,
                "ome_accel::leaf_aabbs_pool",
                max_leaves as u64 * size_of::<LeafAabb>() as u64,
                /* copy_src */ true,
            ),
            primitives_pool: make_storage(
                device,
                "ome_accel::primitives_pool",
                max_primitives as u64 * primitive_stride as u64,
                /* copy_src */ false,
            ),
            tlas_nodes: make_storage(
                device,
                "ome_accel::tlas_nodes",
                2 * max_chunks as u64 * size_of::<BvhNode>() as u64,
                /* copy_src */ true,
            ),
            tlas_uniforms: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ome_accel::tlas_uniforms"),
                size: size_of::<TlasUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            // COPY_SRC enabled ONLY for test-only readback paths.
            // wgpu validates COPY_SRC against actual submitted ops, so
            // the flag is free in production submits (which never use
            // it). NEVER chain COPY_SRC + readback in production code
            // — that's a CPU readback in the hot path, banned by the
            // GPU-driven invariant.
            tlas_mortons: make_storage(
                device,
                "ome_accel::tlas_mortons",
                tlas_per_leaf,
                /* copy_src */ true,
            ),
            // COPY_SRC enabled ONLY for test-only readback paths.
            // wgpu validates COPY_SRC against actual submitted ops, so
            // the flag is free in production submits (which never use
            // it). NEVER chain COPY_SRC + readback in production code
            // — that's a CPU readback in the hot path, banned by the
            // GPU-driven invariant.
            tlas_sorted_indices: make_storage(
                device,
                "ome_accel::tlas_sorted_indices",
                tlas_per_leaf,
                /* copy_src */ true,
            ),
            // COPY_SRC enabled ONLY for test-only readback paths.
            // wgpu validates COPY_SRC against actual submitted ops, so
            // the flag is free in production submits (which never use
            // it). NEVER chain COPY_SRC + readback in production code
            // — that's a CPU readback in the hot path, banned by the
            // GPU-driven invariant.
            tlas_parents: make_storage(
                device,
                "ome_accel::tlas_parents",
                tlas_per_node,
                /* copy_src */ true,
            ),
            // COPY_SRC enabled ONLY for test-only readback paths.
            // wgpu validates COPY_SRC against actual submitted ops, so
            // the flag is free in production submits (which never use
            // it). NEVER chain COPY_SRC + readback in production code
            // — that's a CPU readback in the hot path, banned by the
            // GPU-driven invariant.
            tlas_done: make_storage(
                device,
                "ome_accel::tlas_done",
                tlas_per_node,
                /* copy_src */ true,
            ),
            // COPY_SRC enabled ONLY for test-only readback paths.
            tlas_live_chunk_indices: make_storage(
                device,
                "ome_accel::tlas_live_chunk_indices",
                tlas_per_leaf,
                /* copy_src */ true,
            ),
        }
    }
}

fn make_storage(
    device: &wgpu::Device,
    label: &'static str,
    size: u64,
    copy_src: bool,
) -> wgpu::Buffer {
    let mut usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
    if copy_src {
        usage |= wgpu::BufferUsages::COPY_SRC;
    }
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tlas_per_leaf_scratch_size_matches_u32_per_chunk() {
        // Locks the formula `4 B × max_chunks` for the per-leaf TLAS
        // scratch slots (`mortons`, `sorted_indices`). Independent of
        // any wgpu device — pure arithmetic check.
        assert_eq!(tlas_per_leaf_scratch_size_bytes(0), 0);
        assert_eq!(tlas_per_leaf_scratch_size_bytes(1), 4);
        assert_eq!(tlas_per_leaf_scratch_size_bytes(1024), 4 * 1024);
        assert_eq!(tlas_per_leaf_scratch_size_bytes(65_536), 4 * 65_536);
    }

    #[test]
    fn tlas_per_node_scratch_size_matches_2n_u32_per_chunk() {
        // Locks the formula `8 B × max_chunks` (== `4 B × 2 × max_chunks`)
        // for the per-node TLAS scratch slots (`parents`, `done`).
        // Karras propagation needs one flag / parent-pointer per node
        // across the flat `2N - 1` tree.
        assert_eq!(tlas_per_node_scratch_size_bytes(0), 0);
        assert_eq!(tlas_per_node_scratch_size_bytes(1), 8);
        assert_eq!(tlas_per_node_scratch_size_bytes(1024), 8 * 1024);
        assert_eq!(tlas_per_node_scratch_size_bytes(65_536), 8 * 65_536);
    }

    #[test]
    fn tlas_scratch_total_at_default_caps_is_28_kib() {
        // 3 per-leaf buffers × 4 B × 1024 (`mortons`,
        // `sorted_indices`, `live_chunk_indices`) + 2 per-node buffers
        // × 8 B × 1024 (`parents`, `done`) = 28 KiB constant VRAM
        // overhead introduced by the TLAS GPU rebuild path.
        const PER_LEAF_BUFFERS: u64 = 3;
        const PER_NODE_BUFFERS: u64 = 2;
        const DEFAULT_MAX_CHUNKS: u32 = 1024;
        let total = PER_LEAF_BUFFERS
            * tlas_per_leaf_scratch_size_bytes(DEFAULT_MAX_CHUNKS)
            + PER_NODE_BUFFERS
                * tlas_per_node_scratch_size_bytes(DEFAULT_MAX_CHUNKS);
        assert_eq!(total, 28 * 1024);
    }
}
