//! Golden CPU/GPU consistency tests for `Bvh::build_gpu` + the
//! refit-vs-CPU-ground-truth tests for `refit_gpu`.
//!
//! Build sizes cover the structural edge cases:
//!
//! - **N = 1**: leaves-only dispatch (no internals). Verifies the
//!   `n >= 2` orchestrator guard.
//! - **N = 2**: smallest non-trivial Karras tree (1 internal +
//!   2 leaves). Catches off-by-one in `range_and_split` for the
//!   lower bound — N = 8 + masks this.
//! - **N = 8**: balanced grid, sub-tile (one onesweep partition).
//! - **N = 100**: random AABBs, asymmetric split, sub-tile.
//! - **N = 1024**: balanced 32 × 32 grid, exactly one onesweep
//!   partition.
//! - **N = 65 000**: 22 onesweep partitions + ~16 levels of AABB
//!   propagation. Stress-tests the decoupled-lookback chained scan
//!   and the AABB iteration count, plus the buffer growth path.
//!
//! Refit sizes mirror the build sizes (minus the 65k stress test):
//! build → translate every AABB by a small delta → refit → readback.
//! Compared byte-exact against a CPU ground-truth that walks the
//! captured topology with the new AABBs.
//!
//! Each test uses a deterministic seed so failures are reproducible.

use crate::aabb::Aabb;
use crate::bvh::Bvh;
use crate::gpu::builder::{BvhGpuBuilder, test_device};
use crate::node::BvhNode;

use super::full::build_gpu;
use super::refit::refit_gpu;

use glam::Vec3;

fn aabb_at(centre: Vec3, half: f32) -> Aabb {
    Aabb::from_centre(centre, Vec3::splat(half))
}

/// Cheap deterministic LCG — the same constants used elsewhere in
/// the ome_bvh tests, so reproductions match.
fn lcg(state: &mut u32) -> f32 {
    *state = state.wrapping_mul(1103515245).wrapping_add(12345);
    (*state >> 16) as f32 / 32768.0
}

fn random_items(n: u32, seed: u32, world_size: f32) -> Vec<(u32, Aabb)> {
    let mut state = seed;
    (0..n)
        .map(|i| {
            let centre = Vec3::new(lcg(&mut state), lcg(&mut state), lcg(&mut state))
                * world_size;
            (i, Aabb::from_centre(centre, Vec3::splat(0.2)))
        })
        .collect()
}

fn assert_gpu_matches_cpu(gpu: &Bvh<u32>, cpu: &Bvh<u32>, label: &str) {
    assert_eq!(
        gpu.nodes.len(),
        cpu.nodes.len(),
        "[{label}] node count: gpu={} cpu={}",
        gpu.nodes.len(),
        cpu.nodes.len()
    );
    for (i, (g, c)) in gpu.nodes.iter().zip(cpu.nodes.iter()).enumerate() {
        assert_eq!(
            g, c,
            "[{label}] node[{i}] diverges:\n  gpu: {g:?}\n  cpu: {c:?}"
        );
    }
    assert_eq!(
        gpu.leaves, cpu.leaves,
        "[{label}] leaves payload mismatch"
    );
}

fn run_pair(items: Vec<(u32, Aabb)>, label: &str) {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("ome_bvh::gpu::build: no GPU adapter — skipping {label}");
        return;
    };
    let mut builder = BvhGpuBuilder::new(&device, &queue, None);
    let cpu = Bvh::build(items.clone());
    let build = build_gpu::<u32>(&mut builder, &device, &queue, items);
    let gpu = build.block_on(&device).expect("GPU build failed");
    assert_gpu_matches_cpu(&gpu, &cpu, label);
}

#[test]
fn build_gpu_matches_cpu_n_1() {
    // Smallest non-empty tree: a single leaf, no internals. Hits
    // the `n >= 2` orchestrator guard around sort + internal +
    // AABB passes; verifies the leaves-only dispatch resolves
    // cleanly.
    let items = vec![(0u32, aabb_at(Vec3::ZERO, 1.0))];
    run_pair(items, "n=1");
}

#[test]
fn build_gpu_matches_cpu_n_2() {
    // Smallest Karras non-trivial tree: 1 internal at idx 0, 2
    // leaves at idx 1, 2. Catches off-by-one bugs in
    // `range_and_split` for the lower bound — N=8 masks them
    // because the asymmetry tends to fall on the upper side.
    let items = vec![
        (0u32, aabb_at(Vec3::ZERO, 0.5)),
        (1u32, aabb_at(Vec3::splat(10.0), 0.5)),
    ];
    run_pair(items, "n=2");
}

#[test]
fn build_gpu_matches_cpu_n_8() {
    // Balanced linear grid (one onesweep partition).
    let items: Vec<(u32, Aabb)> = (0..8u32)
        .map(|i| (i, aabb_at(Vec3::new(i as f32, 0.0, 0.0), 0.4)))
        .collect();
    run_pair(items, "n=8");
}

#[test]
fn build_gpu_matches_cpu_n_100() {
    // Random AABBs in a 10×10×10 box — asymmetric split inside one
    // onesweep partition.
    run_pair(random_items(100, 0xc0ffee01, 10.0), "n=100");
}

#[test]
fn build_gpu_matches_cpu_n_1024() {
    // 32×32 grid — exactly one onesweep partition (ITEMS_PER_TILE
    // = 3072 > 1024). Balanced tree with depth ⌈log₂ 1024⌉ = 10.
    let items: Vec<(u32, Aabb)> = (0..1024u32)
        .map(|i| {
            let x = (i % 32) as f32;
            let y = (i / 32) as f32;
            (i, aabb_at(Vec3::new(x, y, 0.0), 0.4))
        })
        .collect();
    run_pair(items, "n=1024");
}

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
fn run_refit_pair(
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
fn perturb_translate(items: &[(u32, Aabb)], delta: Vec3) -> Vec<(u32, Aabb)> {
    items
        .iter()
        .map(|&(id, a)| (id, Aabb::from_centre(a.center() + delta, (a.max - a.min) * 0.5)))
        .collect()
}

#[test]
fn refit_gpu_matches_cpu_n_1() {
    let items = vec![(0u32, aabb_at(Vec3::ZERO, 1.0))];
    let moved = vec![(0u32, aabb_at(Vec3::splat(0.3), 1.0))];
    run_refit_pair(items, moved, "refit_n=1");
}

#[test]
fn refit_gpu_matches_cpu_n_2() {
    let items = vec![
        (0u32, aabb_at(Vec3::ZERO, 0.5)),
        (1u32, aabb_at(Vec3::splat(10.0), 0.5)),
    ];
    let moved = vec![
        (0u32, aabb_at(Vec3::splat(0.05), 0.55)),
        (1u32, aabb_at(Vec3::splat(10.05), 0.45)),
    ];
    run_refit_pair(items, moved, "refit_n=2");
}

#[test]
fn refit_gpu_matches_cpu_n_8() {
    let items: Vec<(u32, Aabb)> = (0..8u32)
        .map(|i| (i, aabb_at(Vec3::new(i as f32, 0.0, 0.0), 0.4)))
        .collect();
    let moved = perturb_translate(&items, Vec3::new(0.05, 0.02, -0.03));
    run_refit_pair(items, moved, "refit_n=8");
}

#[test]
fn refit_gpu_matches_cpu_n_1024() {
    let items: Vec<(u32, Aabb)> = (0..1024u32)
        .map(|i| {
            let x = (i % 32) as f32;
            let y = (i / 32) as f32;
            (i, aabb_at(Vec3::new(x, y, 0.0), 0.4))
        })
        .collect();
    let moved = perturb_translate(&items, Vec3::new(0.05, -0.05, 0.02));
    run_refit_pair(items, moved, "refit_n=1024");
}

#[test]
fn refit_gpu_matches_cpu_random_n_100() {
    let items = random_items(100, 0xc0ffee01, 10.0);
    let moved = perturb_translate(&items, Vec3::new(0.03, 0.04, -0.02));
    run_refit_pair(items, moved, "refit_n=100");
}

#[test]
fn build_gpu_matches_cpu_n_65000() {
    // 65 000 random items — 22 onesweep partitions
    // (ceil(65000/3072) = 22), AABB propagation depth ~16. Stress
    // tests:
    //   - decoupled-lookback chained scan across partitions
    //   - buffer growth path (initial cap 1024 → next_pow2 65536)
    //   - AABB iteration count `⌈log₂ N⌉ + 4 = 20`
    run_pair(random_items(65_000, 0xfeedface, 1000.0), "n=65000");
}
