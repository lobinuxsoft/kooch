//! Pass 2 (leaves) writes the correct AABB + chunk_idx + done flag.

use super::helpers::*;
use crate::accel::tlas::encode_live;
use crate::gpu::builder::test_device;
use crate::gpu::tlas_lbvh::TlasGpuBuilder;
use crate::gpu::types::GpuSceneBounds;
use crate::node::BvhNode;

#[test]
fn tlas_leaves_writes_correct_aabb_and_chunk_idx() {
    // Pins the TLAS leaf encoding: aabb pulled from the chunk
    // descriptor (NOT from any inflated bound), `left = 0`,
    // `right_or_count = chunk_idx | BVH_LEAF_FLAG` (encode_live).
    // Also asserts every `tlas_done[k]` is set so the upcoming AABB
    // propagation pass sees finalised leaves.
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("ome_bvh::gpu::tlas_lbvh: no GPU adapter — skipping");
        return;
    };
    let (descs, aabbs, chunk_descs_buf, mortons_buf, sorted_indices_buf, n) =
        prepare_inputs(&device);
    let (tlas_nodes_buf, tlas_done_buf) = prepare_leaf_outputs(&device, n);

    let scene = GpuSceneBounds::from_aabbs(&aabbs);
    let mut builder = TlasGpuBuilder::new(&device, None);
    builder.ensure_capacity(&device, n as u64);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("tlas_leaves_test_encoder"),
    });
    builder.dispatch_morton(
        &device,
        &queue,
        &mut encoder,
        &chunk_descs_buf,
        &mortons_buf,
        scene,
        n,
    );
    builder.dispatch_sort(
        &device,
        &queue,
        &mut encoder,
        &mortons_buf,
        &sorted_indices_buf,
        n,
    );
    builder.dispatch_leaves(
        &device,
        &mut encoder,
        &tlas_nodes_buf,
        &sorted_indices_buf,
        &chunk_descs_buf,
        &tlas_done_buf,
        n,
    );
    queue.submit(std::iter::once(encoder.finish()));

    let nodes_full: Vec<BvhNode> = readback_pod(&device, &queue, &tlas_nodes_buf, 2 * n);
    let sorted_indices = readback_u32(&device, &queue, &sorted_indices_buf, n);
    let dones = readback_u32(&device, &queue, &tlas_done_buf, n);

    let leaf_offset = (n - 1) as usize;
    for k in 0..n as usize {
        let leaf = nodes_full[leaf_offset + k];
        let chunk_idx = sorted_indices[k];
        let desc = &descs[chunk_idx as usize];
        assert_eq!(
            leaf.aabb_min, desc.aabb_min,
            "leaf[{k}] aabb_min should match chunk_descriptors[{chunk_idx}]",
        );
        assert_eq!(
            leaf.aabb_max, desc.aabb_max,
            "leaf[{k}] aabb_max should match chunk_descriptors[{chunk_idx}]",
        );
        assert_eq!(leaf.left, 0, "TLAS leaf[{k}] left field must be 0");
        assert_eq!(
            leaf.right_or_count,
            encode_live(chunk_idx),
            "leaf[{k}] payload must encode chunk_idx={chunk_idx} via encode_live",
        );
        assert_eq!(
            dones[k], 1,
            "tlas_done[{k}] must be 1 after leaves dispatch",
        );
    }
}
