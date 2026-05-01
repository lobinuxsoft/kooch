//! Pass 4 (AABB propagation) + dispatch_rebuild end-to-end.

use super::helpers::*;
use crate::accel::buffers::AccelBuffers;
use crate::gpu::builder::test_device;
use crate::gpu::tlas_lbvh::TlasGpuBuilder;
use crate::node::{BVH_VALUE_MASK, BvhNode};

const TEST_MAX_CHUNKS: u32 = 32;

fn fresh_accel_buffers(device: &wgpu::Device) -> AccelBuffers {
    // Caps stay generous enough that the BLAS pools (unused by these
    // TLAS-only tests) don't trip min_binding_size validation.
    AccelBuffers::new(
        device,
        TEST_MAX_CHUNKS, // max_chunks
        TEST_MAX_CHUNKS * 2, // max_nodes
        TEST_MAX_CHUNKS, // max_leaves
        TEST_MAX_CHUNKS * 4, // max_primitives
        16, // primitive_stride
    )
}

#[test]
fn tlas_aabb_propagation_envelopes_children() {
    // The full TLAS rebuild populates every internal's AABB with the
    // tight union of its descendants. This test asserts the
    // bottom-up convergence holds for n=16: every internal's
    // [aabb_min, aabb_max] tightly contains both children's bounds.
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("ome_bvh::gpu::tlas_lbvh: no GPU adapter — skipping");
        return;
    };
    let accel_buffers = fresh_accel_buffers(&device);
    let descs: Vec<crate::accel::descriptor::ChunkDescriptor> = TEST_CENTRES
        .iter()
        .map(|c| descriptor_for(*c, 0.4))
        .collect();
    let n = descs.len() as u32;
    queue.write_buffer(
        &accel_buffers.chunk_descriptors,
        0,
        bytemuck::cast_slice(&descs),
    );

    let mut builder = TlasGpuBuilder::new(&device, None);
    builder.ensure_capacity(&device, n as u64);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("tlas_rebuild_aabb_test_encoder"),
    });
    builder.dispatch_rebuild(&device, &queue, &mut encoder, &accel_buffers, &descs, n);
    queue.submit(std::iter::once(encoder.finish()));

    let nodes_full: Vec<BvhNode> =
        readback_pod(&device, &queue, &accel_buffers.tlas_nodes, 2 * n);
    let total_nodes = (2 * n - 1) as usize;

    for i in 0..(n - 1) as usize {
        let internal = nodes_full[i];
        let left_idx = internal.left as usize;
        let right_idx = (internal.right_or_count & BVH_VALUE_MASK) as usize;
        assert!(
            left_idx < total_nodes && right_idx < total_nodes,
            "internal[{i}] children out of range: left={left_idx} right={right_idx}",
        );
        let left = nodes_full[left_idx];
        let right = nodes_full[right_idx];

        for axis in 0..3 {
            assert!(
                internal.aabb_min[axis] <= left.aabb_min[axis] + 1e-5,
                "internal[{i}].aabb_min[{axis}]={} must envelope left child {}",
                internal.aabb_min[axis],
                left.aabb_min[axis],
            );
            assert!(
                internal.aabb_min[axis] <= right.aabb_min[axis] + 1e-5,
                "internal[{i}].aabb_min[{axis}]={} must envelope right child {}",
                internal.aabb_min[axis],
                right.aabb_min[axis],
            );
            assert!(
                internal.aabb_max[axis] + 1e-5 >= left.aabb_max[axis],
                "internal[{i}].aabb_max[{axis}]={} must envelope left child {}",
                internal.aabb_max[axis],
                left.aabb_max[axis],
            );
            assert!(
                internal.aabb_max[axis] + 1e-5 >= right.aabb_max[axis],
                "internal[{i}].aabb_max[{axis}]={} must envelope right child {}",
                internal.aabb_max[axis],
                right.aabb_max[axis],
            );
        }
    }

    // Sanity: the root (tlas_nodes[0]) must envelope every leaf.
    let root = nodes_full[0];
    let leaf_offset = (n - 1) as usize;
    for k in 0..n as usize {
        let leaf = nodes_full[leaf_offset + k];
        for axis in 0..3 {
            assert!(
                root.aabb_min[axis] <= leaf.aabb_min[axis] + 1e-5,
                "root.aabb_min[{axis}]={} must envelope leaf[{k}].aabb_min={}",
                root.aabb_min[axis],
                leaf.aabb_min[axis],
            );
            assert!(
                root.aabb_max[axis] + 1e-5 >= leaf.aabb_max[axis],
                "root.aabb_max[{axis}]={} must envelope leaf[{k}].aabb_max={}",
                root.aabb_max[axis],
                leaf.aabb_max[axis],
            );
        }
    }
}

#[test]
fn tlas_rebuild_end_to_end_n_equals_1() {
    // Single-chunk pool: the leaf IS the root. dispatch_rebuild must
    // populate `tlas_nodes[0]` with the chunk's AABB and skip the
    // internal + aabb passes (no topology to build).
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("ome_bvh::gpu::tlas_lbvh: no GPU adapter — skipping");
        return;
    };
    let accel_buffers = fresh_accel_buffers(&device);
    let desc = descriptor_for(glam::Vec3::new(3.0, 4.0, 5.0), 0.4);
    let descs = vec![desc];
    let n = 1u32;
    queue.write_buffer(
        &accel_buffers.chunk_descriptors,
        0,
        bytemuck::cast_slice(&descs),
    );

    let mut builder = TlasGpuBuilder::new(&device, None);
    builder.ensure_capacity(&device, n as u64);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("tlas_rebuild_n1_test_encoder"),
    });
    builder.dispatch_rebuild(&device, &queue, &mut encoder, &accel_buffers, &descs, n);
    queue.submit(std::iter::once(encoder.finish()));

    let nodes: Vec<BvhNode> =
        readback_pod(&device, &queue, &accel_buffers.tlas_nodes, 2 * n);
    let leaf = nodes[0];
    assert_eq!(leaf.aabb_min, desc.aabb_min, "n=1 leaf aabb_min");
    assert_eq!(leaf.aabb_max, desc.aabb_max, "n=1 leaf aabb_max");
    assert_eq!(leaf.left, 0, "n=1 leaf left=0");
    assert_eq!(
        leaf.right_or_count,
        0x80000000, // chunk_idx=0 | BVH_LEAF_FLAG
        "n=1 leaf payload encodes chunk_idx=0",
    );
}

#[test]
fn tlas_rebuild_end_to_end_empty_pool() {
    // n=0: dispatch_rebuild early-returns. Caller is responsible for
    // the sentinel zero-write (handled by `accel::tlas::rebuild` in
    // commit 8). Just asserts no panic from the empty path.
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("ome_bvh::gpu::tlas_lbvh: no GPU adapter — skipping");
        return;
    };
    let accel_buffers = fresh_accel_buffers(&device);
    let descs: Vec<crate::accel::descriptor::ChunkDescriptor> = vec![];
    let mut builder = TlasGpuBuilder::new(&device, None);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("tlas_rebuild_empty_test_encoder"),
    });
    builder.dispatch_rebuild(&device, &queue, &mut encoder, &accel_buffers, &descs, 0);
    queue.submit(std::iter::once(encoder.finish()));
    // No assertion beyond "doesn't panic / doesn't crash the device".
}
