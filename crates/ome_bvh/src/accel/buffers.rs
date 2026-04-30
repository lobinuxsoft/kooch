//! Pre-allocated GPU buffers backing the TLAS+BLAS pool.
//!
//! Every buffer is sized at `OmeAccel::new` and never reallocated —
//! the hot path runs `device.queue().write_buffer` slice writes only.
//! `TlasUniforms` is the lone uniform binding; everything else is
//! `STORAGE | COPY_DST`. `COPY_SRC` is enabled on the BLAS pools so
//! the regression tests can read back without parallel CPU mirrors.

use crate::accel::descriptor::{ChunkDescriptor, TlasUniforms};
use crate::leaf::LeafAabb;
use crate::node::BvhNode;

/// All six pool buffers, owned together so the bind group can be
/// rebuilt in a single closure. Mirrors the layout the issue body
/// pins for bind group 1 bindings 5..=10.
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

use std::mem::size_of;
