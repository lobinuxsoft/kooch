//! Pass 3 (internal Karras) topology validity for n=16.

use super::helpers::*;
use crate::gpu::builder::test_device;
use crate::gpu::tlas_lbvh::TlasGpuBuilder;
use crate::gpu::types::GpuSceneBounds;
use crate::node::BvhNode;

#[test]
fn tlas_internal_writes_valid_topology() {
    // Pins the Karras TLAS topology invariants:
    //   1. Every internal node's `right_or_count` does NOT have
    //      BVH_LEAF_FLAG set (it's a node index, not an encoded leaf).
    //   2. Both children of every internal land in the valid node
    //      range `[0, 2N - 1)`.
    //   3. For every internal i, `parents[role_idx(left)] == i` and
    //      `parents[role_idx(right)] == i` (parent-child consistency).
    //   4. Exactly one role_idx position is NOT pointed at by any
    //      internal — that's the root node.
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("ome_bvh::gpu::tlas_lbvh: no GPU adapter — skipping");
        return;
    };
    let inputs = prepare_inputs(&device);
    let n = inputs.n;
    let (tlas_nodes_buf, tlas_done_buf) = prepare_leaf_outputs(&device, n);
    let tlas_parents_buf = prepare_parents(&device, n);

    let scene = GpuSceneBounds::from_aabbs(&inputs.aabbs);
    let mut builder = TlasGpuBuilder::new(&device, None);
    builder.ensure_capacity(&device, n as u64);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("tlas_internal_test_encoder"),
    });
    builder.dispatch_morton(
        &device,
        &queue,
        &mut encoder,
        &inputs.chunk_descs_buf,
        &inputs.mortons_buf,
        &inputs.live_chunk_indices_buf,
        scene,
        n,
    );
    builder.dispatch_sort(
        &device,
        &queue,
        &mut encoder,
        &inputs.mortons_buf,
        &inputs.sorted_indices_buf,
        n,
    );
    builder.dispatch_leaves(
        &device,
        &mut encoder,
        &tlas_nodes_buf,
        &inputs.sorted_indices_buf,
        &inputs.chunk_descs_buf,
        &tlas_done_buf,
        &inputs.live_chunk_indices_buf,
        n,
    );
    builder.dispatch_internal(
        &device,
        &mut encoder,
        &tlas_nodes_buf,
        &inputs.mortons_buf,
        &tlas_parents_buf,
        &tlas_done_buf,
        n,
    );
    queue.submit(std::iter::once(encoder.finish()));

    let nodes_full: Vec<BvhNode> = readback_pod(&device, &queue, &tlas_nodes_buf, 2 * n);
    let parents = readback_u32(&device, &queue, &tlas_parents_buf, 2 * n);

    let total_nodes = 2 * n - 1;
    let mut covered = vec![false; (2 * n) as usize];

    for i in 0..(n - 1) {
        let internal = nodes_full[i as usize];
        assert_eq!(
            internal.right_or_count & 0x80000000,
            0,
            "internal[{i}] must NOT have BVH_LEAF_FLAG set in right_or_count",
        );
        let left = internal.left;
        let right = internal.right_or_count;
        assert!(
            left < total_nodes,
            "internal[{i}].left = {left} out of valid range [0, {total_nodes})",
        );
        assert!(
            right < total_nodes,
            "internal[{i}].right = {right} out of valid range [0, {total_nodes})",
        );

        let left_role = role_idx(left, n);
        let right_role = role_idx(right, n);
        assert_eq!(
            parents[left_role as usize], i,
            "parents[role_idx(left={left})={left_role}] must equal i={i}",
        );
        assert_eq!(
            parents[right_role as usize], i,
            "parents[role_idx(right={right})={right_role}] must equal i={i}",
        );

        covered[left_role as usize] = true;
        covered[right_role as usize] = true;
    }

    // Root invariant: exactly one role_idx slot in the valid range
    // [0, 2N - 1) is uncovered (no internal points to it as a child).
    // That's the root. Slot 2N - 1 is the unused tail of the buffer
    // (Karras tree has 2N - 1 nodes; we round to 2N) — exclude it.
    let uncovered: Vec<u32> = (0..total_nodes).filter(|i| !covered[*i as usize]).collect();
    assert_eq!(
        uncovered.len(),
        1,
        "exactly one role_idx slot must be uncovered (the root) — \
         got {} uncovered: {:?}",
        uncovered.len(),
        uncovered,
    );
    // The Karras root is the internal at tlas_nodes[0] → role_idx = N.
    let expected_root_role = n;
    assert_eq!(
        uncovered[0], expected_root_role,
        "root role_idx should be N={expected_root_role} (internal i=0)",
    );
}
