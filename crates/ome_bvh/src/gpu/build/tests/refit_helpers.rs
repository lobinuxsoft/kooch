use crate::aabb::Aabb;
use crate::gpu::builder::{test_device, BvhGpuBuilder};
use crate::node::BvhNode;

use super::super::full::build_gpu;
use super::super::refit::refit_gpu;

use glam::Vec3;

/// CPU ground-truth refit. Walks the topology in `nodes` and
/// rewrites every node's AABB from the new `aabbs` (indexed
/// through the original-to-sorted permutation captured at build
/// time). Mirrors the GPU's bottom-up multi-dispatch — same
/// `done[]` book-keeping, same iteration cap.
fn cpu_refit_ground_truth(
    nodes: &[BvhNode],
    sorted_indices: &[u32],
    new_aabbs: &[Aabb],
) -> Vec<BvhNode> {
    let n = sorted_indices.len() as u32;
    let mut out = nodes.to_vec();

    if n == 0 {
        return out;
    }

    // Phase 1: rewrite leaves with new AABBs through the
    // build-time permutation (`sorted_indices[k]` = original
    // index at sorted position k).
    let leaf_offset: usize = if n == 1 { 0 } else { (n - 1) as usize };
    for k in 0..n as usize {
        let leaf_idx = leaf_offset + k;
        let original = sorted_indices[k] as usize;
        let aabb = new_aabbs[original];
        out[leaf_idx].aabb_min = aabb.min.to_array();
        out[leaf_idx].aabb_max = aabb.max.to_array();
    }

    if n < 2 {
        return out;
    }

    // Phase 2: bottom-up internal propagation, identical
    // semantics to `karras_aabb.wgsl`.
    let total = (2 * n - 1) as usize;
    let mut done = vec![false; total];
    for k in 0..n as usize {
        done[(n - 1) as usize + k] = true;
    }
    // Match the GPU's iteration cap exactly so this catches the
    // same convergence problems the GPU would.
    let max_iters = crate::gpu::lbvh::aabb_iterations(n) as usize;
    for _ in 0..max_iters {
        let mut changed = false;
        for i in 0..(n - 1) as usize {
            if done[i] {
                continue;
            }
            let left = out[i].left as usize;
            let right = (out[i].right_or_count & crate::node::BVH_VALUE_MASK) as usize;
            if !done[left] || !done[right] {
                continue;
            }
            let lmin = Vec3::from_array(out[left].aabb_min);
            let lmax = Vec3::from_array(out[left].aabb_max);
            let rmin = Vec3::from_array(out[right].aabb_min);
            let rmax = Vec3::from_array(out[right].aabb_max);
            out[i].aabb_min = lmin.min(rmin).to_array();
            out[i].aabb_max = lmax.max(rmax).to_array();
            done[i] = true;
            changed = true;
        }
        if !changed {
            break;
        }
    }
    // Ground-truth invariant: every internal must converge under
    // the same iteration budget the GPU uses. Otherwise the GPU's
    // own debug invariant would have already panicked.
    assert!(
        done.iter().take((n - 1) as usize).all(|&d| d),
        "CPU refit ground-truth failed to converge — bench seed exposes a tree the GPU \
         slack also can't handle. Re-evaluate AABB_ITERATION_SLACK before shipping."
    );
    out
}

/// Read back the builder's `nodes_buffer` for assertions.
fn readback_builder_nodes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    builder: &BvhGpuBuilder,
    n: u32,
) -> Vec<BvhNode> {
    let total = (2 * n - 1) as u64;
    let bytes = total * std::mem::size_of::<BvhNode>() as u64;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_bvh::refit_test_nodes_readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("ome_bvh::refit_test_nodes_readback_encoder"),
    });
    encoder.copy_buffer_to_buffer(builder.nodes_buffer(), 0, &staging, 0, bytes);
    queue.submit(std::iter::once(encoder.finish()));
    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        tx.send(r).ok();
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("device poll");
    rx.recv().expect("map sender").expect("map result");
    let data = slice.get_mapped_range();
    let v: Vec<BvhNode> = bytemuck::cast_slice::<u8, BvhNode>(&data).to_vec();
    drop(data);
    staging.unmap();
    v
}

fn readback_sorted_indices(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    builder: &BvhGpuBuilder,
    n: u32,
) -> Vec<u32> {
    let bytes = (n as u64) * 4;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_bvh::refit_test_indices_readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("ome_bvh::refit_test_indices_readback_encoder"),
    });
    encoder.copy_buffer_to_buffer(builder.sorted_indices_buffer(), 0, &staging, 0, bytes);
    queue.submit(std::iter::once(encoder.finish()));
    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        tx.send(r).ok();
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("device poll");
    rx.recv().expect("map sender").expect("map result");
    let data = slice.get_mapped_range();
    let v: Vec<u32> = bytemuck::cast_slice::<u8, u32>(&data).to_vec();
    drop(data);
    staging.unmap();
    v
}

/// Drive a build to completion, then a refit, asserting the
/// builder's resulting `nodes_buffer` is byte-identical to the
/// CPU ground-truth refit.
pub(super) fn run_refit_pair(
    items_v0: Vec<(u32, Aabb)>,
    items_v1: Vec<(u32, Aabb)>,
    label: &str,
) {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("ome_bvh::gpu::build: no GPU adapter — skipping refit {label}");
        return;
    };
    assert_eq!(items_v0.len(), items_v1.len(), "refit cardinality must match");
    let n = items_v0.len() as u32;
    let mut builder = BvhGpuBuilder::new(&device, &queue, None);

    // Initial build.
    let build = build_gpu::<u32>(&mut builder, &device, &queue, items_v0.clone());
    let _ = build.block_on(&device).expect("initial build failed");

    // Capture the topology + permutation that the refit must preserve.
    let topology_v0 = readback_builder_nodes(&device, &queue, &builder, n);
    let sorted_indices_v0 = readback_sorted_indices(&device, &queue, &builder, n);

    // Refit.
    let refit = refit_gpu::<u32>(&mut builder, &device, &queue, items_v1.clone());
    refit.block_on(&device).expect("refit failed");

    // GPU result.
    let gpu_after_refit = readback_builder_nodes(&device, &queue, &builder, n);

    // CPU ground-truth refit over the captured topology.
    let new_aabbs: Vec<Aabb> = items_v1.iter().map(|(_, a)| *a).collect();
    let cpu_truth =
        cpu_refit_ground_truth(&topology_v0, &sorted_indices_v0, &new_aabbs);

    for (i, (g, c)) in gpu_after_refit.iter().zip(cpu_truth.iter()).enumerate() {
        assert_eq!(
            g, c,
            "[{label}] node[{i}] GPU/CPU refit diverge:\n  gpu: {g:?}\n  cpu: {c:?}"
        );
    }
}

/// Tiny perturbation: every AABB shifts by the same small delta.
/// Topology survives — Morton codes for the centres remain in the
/// same sort order; the refit must produce bit-exact AABBs.
pub(super) fn perturb_translate(items: &[(u32, Aabb)], delta: Vec3) -> Vec<(u32, Aabb)> {
    items
        .iter()
        .map(|&(id, a)| (id, Aabb::from_centre(a.center() + delta, (a.max - a.min) * 0.5)))
        .collect()
}
