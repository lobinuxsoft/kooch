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

/// Byte size of one TLAS scratch slot — `morton`, `sorted_indices`,
/// `parents`, and `done` are each a flat `u32` per chunk.
const TLAS_SCRATCH_ENTRY_BYTES: u64 = size_of::<u32>() as u64;

/// Total byte size of a single TLAS scratch buffer for a pool of at
/// most `max_chunks` chunks. Pulled out of [`AccelBuffers::new`] so a
/// CPU-only unit test can lock the formula without spinning up a
/// `wgpu::Device`.
pub(crate) const fn tlas_scratch_size_bytes(max_chunks: u32) -> u64 {
    max_chunks as u64 * TLAS_SCRATCH_ENTRY_BYTES
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
    /// Per-chunk Morton code, written by `tlas_morton.wgsl`.
    pub tlas_mortons: wgpu::Buffer,
    /// Onesweep-sorted chunk indices (input to `tlas_leaves.wgsl`).
    pub tlas_sorted_indices: wgpu::Buffer,
    /// Parent pointer per TLAS node (written by `tlas_internal.wgsl`).
    pub tlas_parents: wgpu::Buffer,
    /// `done[node]` flag for the TLAS AABB propagation pass.
    pub tlas_done: wgpu::Buffer,
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
        let tlas_scratch = tlas_scratch_size_bytes(max_chunks);
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
            tlas_mortons: make_storage(
                device,
                "ome_accel::tlas_mortons",
                tlas_scratch,
                /* copy_src */ false,
            ),
            tlas_sorted_indices: make_storage(
                device,
                "ome_accel::tlas_sorted_indices",
                tlas_scratch,
                /* copy_src */ false,
            ),
            tlas_parents: make_storage(
                device,
                "ome_accel::tlas_parents",
                tlas_scratch,
                /* copy_src */ false,
            ),
            tlas_done: make_storage(
                device,
                "ome_accel::tlas_done",
                tlas_scratch,
                /* copy_src */ false,
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
    fn tlas_scratch_size_matches_u32_per_chunk() {
        // Locks the formula `4 B × max_chunks` for every TLAS scratch
        // slot. Independent of any wgpu device — pure arithmetic check.
        assert_eq!(tlas_scratch_size_bytes(0), 0);
        assert_eq!(tlas_scratch_size_bytes(1), 4);
        assert_eq!(tlas_scratch_size_bytes(1024), 4 * 1024);
        assert_eq!(tlas_scratch_size_bytes(65_536), 4 * 65_536);
    }

    #[test]
    fn tlas_scratch_total_at_default_caps_is_16_kib() {
        // Four scratch buffers × 4 B × 1024 chunks = 16 KiB constant
        // VRAM overhead introduced by the TLAS GPU rebuild path.
        const FOUR_BUFFERS: u64 = 4;
        const DEFAULT_MAX_CHUNKS: u32 = 1024;
        assert_eq!(
            FOUR_BUFFERS * tlas_scratch_size_bytes(DEFAULT_MAX_CHUNKS),
            16 * 1024,
        );
    }
}
