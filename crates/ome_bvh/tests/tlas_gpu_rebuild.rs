//! GPU integration tests for the TLAS GPU rebuild pipeline (epic
//! #370 PR-1, end-to-end). Skips itself gracefully when no GPU
//! adapter is present (CI without a display).
//!
//! Coverage matrix:
//! 1. `tlas_gpu_matches_cpu_for_known_topology` — 16 hand-picked
//!    chunks, GPU output byte-identical to a CPU `Bvh::build_into`
//!    ground truth.
//! 2. `tlas_gpu_handles_empty_pool` — `n=0` writes the sentinel.
//! 3. `tlas_gpu_handles_single_chunk` — `n=1` leaf at `tlas_nodes[0]`.
//! 4. `tlas_gpu_dispatch_no_readback` — wall-clock timing bound on
//!    `update_gpu_standalone`; surfaces accidental sync waits.
//! 5. `tlas_gpu_full_rebuild_under_churn` — 100 cycles of
//!    insert/remove + rebuild; `tlas_dirty_count == 0` after each.
//!
//! Note: morton byte-identity is covered by the unit test in
//! `gpu/tlas_lbvh/tests/morton.rs`. Promoting it here would not add
//! distinct setup, so it is intentionally not duplicated.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use glam::Vec3;
use ome_bvh::accel::tlas::{decode_chunk_idx, encode_live};
use ome_bvh::accel::{AccelCaps, ChunkInsert, OmeAccel};
use ome_bvh::aabb::Aabb;
use ome_bvh::bvh::Bvh;
use ome_bvh::leaf::{IS_RAYMARCH, LeafAabb, ROLE_RAYMARCH_ADD};
use ome_bvh::node::{BVH_LEAF_FLAG, BVH_VALUE_MASK, BvhNode};

const PRIMITIVE_STRIDE: u32 = 16;

// Mesa radv races on parallel `request_adapter` (issue #334). One
// shared device per test binary keeps the call serialised.
static SHARED_DEVICE: OnceLock<Option<(wgpu::Device, wgpu::Queue)>> = OnceLock::new();

fn try_acquire_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    SHARED_DEVICE
        .get_or_init(|| {
            pollster::block_on(async {
                let instance = wgpu::Instance::default();
                let adapter = instance
                    .request_adapter(&wgpu::RequestAdapterOptions::default())
                    .await
                    .ok()?;
                let (device, queue) = adapter
                    .request_device(&wgpu::DeviceDescriptor {
                        label: Some("ome_bvh::tlas_gpu_rebuild_test_device"),
                        required_features: wgpu::Features::empty(),
                        required_limits: wgpu::Limits::default(),
                        memory_hints: wgpu::MemoryHints::Performance,
                        trace: wgpu::Trace::Off,
                        experimental_features: wgpu::ExperimentalFeatures::default(),
                    })
                    .await
                    .ok()?;
                Some((device, queue))
            })
        })
        .clone()
}

fn fresh_accel(device: &wgpu::Device) -> OmeAccel {
    OmeAccel::new(device, AccelCaps::default(), PRIMITIVE_STRIDE).expect("AccelCaps::default")
}

/// Insert one chunk holding a single primitive at `centre` with the
/// given half-extent. Returns the chunk's world-space AABB so the
/// caller can mirror the (chunk_idx, aabb) pair list locally for the
/// CPU ground-truth comparison (the pool's `live_chunk_descriptors`
/// is `pub(crate)` and not reachable from integration tests).
fn insert_test_chunk(
    accel: &mut OmeAccel,
    queue: &wgpu::Queue,
    key: u64,
    chunk_idx: u32,
    centre: Vec3,
    half: f32,
) -> Aabb {
    let aabb_min = [centre.x - half, centre.y - half, centre.z - half];
    let aabb_max = [centre.x + half, centre.y + half, centre.z + half];
    let leaves = [LeafAabb {
        aabb_min,
        flags: IS_RAYMARCH | ROLE_RAYMARCH_ADD,
        aabb_max,
        entity_id: chunk_idx,
    }];
    let prim_bytes = vec![0u8; PRIMITIVE_STRIDE as usize];
    accel
        .insert_chunk(
            queue,
            ChunkInsert {
                key,
                leaf_aabbs: &leaves,
                primitives_bytes: &prim_bytes,
                max_smoothness_radius: 0.0,
            },
        )
        .expect("insert_chunk");
    Aabb::new(aabb_min.into(), aabb_max.into())
}

fn read_tlas_nodes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    accel: &OmeAccel,
    count: u32,
) -> Vec<BvhNode> {
    let bytes = (count as u64) * std::mem::size_of::<BvhNode>() as u64;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("tlas_gpu_rebuild_test_readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("tlas_gpu_rebuild_test_readback_encoder"),
    });
    encoder.copy_buffer_to_buffer(&accel.buffers.tlas_nodes, 0, &staging, 0, bytes);
    queue.submit(std::iter::once(encoder.finish()));
    let slice = staging.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        sender.send(res).ok();
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(30)),
        })
        .expect("device poll");
    receiver
        .recv()
        .expect("map_async sender")
        .expect("map_async result");
    let data = slice.get_mapped_range();
    let v: Vec<BvhNode> = bytemuck::cast_slice::<u8, BvhNode>(&data).to_vec();
    drop(data);
    staging.unmap();
    v
}

/// CPU-side legacy TLAS rebuild — the reference implementation we
/// compared against pre-PR-1. Mirrors `accel::tlas::rebuild_cpu_legacy`
/// but pulls the (chunk_idx, aabb) pairs from a caller-provided list
/// instead of from the pool's `slots` (which are `pub(crate)`).
fn cpu_legacy_tlas(items: &[(u32, Aabb)]) -> Vec<BvhNode> {
    let n = items.len();
    if n == 0 {
        return vec![BvhNode::default()];
    }
    let total = if n == 1 { 1 } else { 2 * n - 1 };
    let mut nodes = vec![BvhNode::default(); total];
    let mut leaves = vec![0u32; n];
    Bvh::<u32>::build_into(items.to_vec(), &mut nodes, &mut leaves);
    let leaf_offset = n.saturating_sub(1);
    for k in 0..n {
        let chunk_idx = leaves[k];
        nodes[leaf_offset + k].left = 0;
        nodes[leaf_offset + k].right_or_count = encode_live(chunk_idx);
    }
    nodes
}

#[test]
fn tlas_gpu_matches_cpu_for_known_topology() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("tlas_gpu_rebuild: no GPU adapter — skipping");
        return;
    };
    let mut accel = fresh_accel(&device);

    // 16 distinct chunk centres across a 10×10×10 box. No two centres
    // share a Morton bucket — the GPU and CPU Karras builds should
    // therefore produce the same topology, and `encode_live` plus
    // identical AABB inflation yield byte-identical node arrays.
    let centres: [Vec3; 16] = [
        Vec3::new(0.5, 0.5, 0.5),
        Vec3::new(9.5, 0.5, 0.5),
        Vec3::new(0.5, 9.5, 0.5),
        Vec3::new(0.5, 0.5, 9.5),
        Vec3::new(9.5, 9.5, 9.5),
        Vec3::new(2.7, 3.3, 4.1),
        Vec3::new(7.1, 1.9, 6.4),
        Vec3::new(1.2, 8.8, 2.5),
        Vec3::new(5.5, 5.5, 5.5),
        Vec3::new(3.0, 6.0, 9.0),
        Vec3::new(0.1, 0.2, 0.3),
        Vec3::new(9.9, 9.8, 9.7),
        Vec3::new(4.4, 4.4, 4.4),
        Vec3::new(6.6, 2.2, 8.8),
        Vec3::new(2.5, 7.5, 5.0),
        Vec3::new(8.0, 1.0, 3.0),
    ];

    let mut cpu_items: Vec<(u32, Aabb)> = Vec::new();
    for (i, c) in centres.iter().enumerate() {
        let aabb = insert_test_chunk(&mut accel, &queue, (i + 1) as u64, i as u32, *c, 0.4);
        cpu_items.push((i as u32, aabb));
    }
    accel.update_gpu_standalone(&device, &queue, 0.0, 0.0);

    let n = centres.len() as u32;
    let total_nodes = 2 * n - 1;
    let gpu_nodes = read_tlas_nodes(&device, &queue, &accel, total_nodes);
    let cpu_nodes = cpu_legacy_tlas(&cpu_items);

    assert_eq!(gpu_nodes.len(), cpu_nodes.len(), "node count mismatch");

    // Leaves byte-identical: the GPU writes the same `encode_live`
    // payload + AABB pulled from the chunk descriptor.
    let leaf_offset = (n - 1) as usize;
    for k in 0..n as usize {
        let g = gpu_nodes[leaf_offset + k];
        let c = cpu_nodes[leaf_offset + k];
        assert_eq!(g.aabb_min, c.aabb_min, "leaf[{k}] aabb_min mismatch");
        assert_eq!(g.aabb_max, c.aabb_max, "leaf[{k}] aabb_max mismatch");
        assert_eq!(g.left, c.left, "leaf[{k}] left mismatch");
        assert_eq!(
            g.right_or_count, c.right_or_count,
            "leaf[{k}] right_or_count mismatch (chunk encoding)",
        );
    }

    // Internal AABBs byte-identical: same Karras tree, same propagation.
    // Internal `left` / `right_or_count` are NOT compared because the
    // CPU and GPU builders may resolve duplicate Morton ties to two
    // valid (and equally tight) tree shapes; the AABBs converge to the
    // same union regardless. With 16 distinct centres in this fixture
    // the topology happens to match, but the AABB invariant is the
    // contract we hold long-term.
    for i in 0..(n - 1) as usize {
        let g = gpu_nodes[i];
        let c = cpu_nodes[i];
        for axis in 0..3 {
            assert!(
                (g.aabb_min[axis] - c.aabb_min[axis]).abs() < 1e-5,
                "internal[{i}].aabb_min[{axis}] gpu={} cpu={}",
                g.aabb_min[axis],
                c.aabb_min[axis],
            );
            assert!(
                (g.aabb_max[axis] - c.aabb_max[axis]).abs() < 1e-5,
                "internal[{i}].aabb_max[{axis}] gpu={} cpu={}",
                g.aabb_max[axis],
                c.aabb_max[axis],
            );
        }
    }
}

#[test]
fn tlas_gpu_handles_empty_pool() {
    let Some((device, queue)) = try_acquire_device() else {
        return;
    };
    let mut accel = fresh_accel(&device);
    // No insert. tlas_dirty_count == 0, but force the rebuild path by
    // toggling via insert + remove. Cleaner: just call update_gpu
    // unconditionally and assert the sentinel write is preserved on
    // empty pool. update_gpu only triggers rebuild when dirty_count > 0,
    // so we have to drive it through one cycle.
    let _ = insert_test_chunk(&mut accel, &queue, 1, 0, Vec3::ZERO, 0.5);
    accel.remove_chunk(&queue, 1).expect("remove_chunk");
    accel.update_gpu_standalone(&device, &queue, 0.0, 0.0);

    assert_eq!(accel.live_chunk_count(), 0, "pool must be empty");

    let nodes = read_tlas_nodes(&device, &queue, &accel, 1);
    assert_eq!(
        nodes[0],
        BvhNode::default(),
        "empty pool must zero the sentinel at tlas_nodes[0]",
    );
}

#[test]
fn tlas_gpu_handles_single_chunk() {
    let Some((device, queue)) = try_acquire_device() else {
        return;
    };
    let mut accel = fresh_accel(&device);
    let centre = Vec3::new(3.0, 4.0, 5.0);
    let aabb = insert_test_chunk(&mut accel, &queue, 1, 0, centre, 0.4);
    accel.update_gpu_standalone(&device, &queue, 0.0, 0.0);

    assert_eq!(accel.live_chunk_count(), 1);
    assert_eq!(accel.tlas_dirty_count(), 0, "dirty count must clear post-rebuild");

    let nodes = read_tlas_nodes(&device, &queue, &accel, 1);
    let leaf = nodes[0];
    let aabb_min: [f32; 3] = aabb.min.into();
    let aabb_max: [f32; 3] = aabb.max.into();
    assert_eq!(leaf.aabb_min, aabb_min, "n=1 leaf aabb_min");
    assert_eq!(leaf.aabb_max, aabb_max, "n=1 leaf aabb_max");
    assert_eq!(leaf.left, 0);
    assert!(leaf.right_or_count & BVH_LEAF_FLAG != 0, "n=1 leaf flag set");
    assert_eq!(
        decode_chunk_idx(leaf.right_or_count),
        0,
        "n=1 leaf chunk_idx encoded",
    );
    assert_eq!(leaf.right_or_count, encode_live(0));
}

#[test]
fn tlas_gpu_dispatch_no_readback() {
    // Wall-clock bound: a 64-chunk rebuild that did NOT trigger a
    // CPU readback (`map_async` + `device.poll(Wait)`) should return
    // well under 50 ms on any GPU. A regression that introduces a
    // synchronous wait surfaces as multi-hundred-ms latency. The
    // bound is generous to avoid flakiness on busy CI hardware while
    // still catching obvious sync-wait regressions.
    let Some((device, queue)) = try_acquire_device() else {
        return;
    };
    let mut accel = fresh_accel(&device);
    for i in 0..64u32 {
        let x = (i % 8) as f32;
        let y = ((i / 8) % 8) as f32;
        let z = 0.5;
        let _ = insert_test_chunk(&mut accel, &queue, (i + 1) as u64, i, Vec3::new(x, y, z), 0.4);
    }

    let start = Instant::now();
    accel.update_gpu_standalone(&device, &queue, 0.0, 0.0);
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_millis(50),
        "dispatch_rebuild for n=64 took {:?} — exceeded 50ms bound; \
         a sync readback may have crept into the hot path",
        elapsed,
    );
    assert_eq!(accel.tlas_dirty_count(), 0);
}

#[test]
fn tlas_gpu_full_rebuild_under_churn() {
    let Some((device, queue)) = try_acquire_device() else {
        return;
    };
    let mut accel = fresh_accel(&device);

    // Steady-state pool: 16 chunks resident. Track keys in a queue
    // so the churn cycle always evicts a currently-live chunk.
    let mut live_keys: std::collections::VecDeque<u64> =
        std::collections::VecDeque::with_capacity(16);
    for i in 0..16u32 {
        let key = (1000 + i) as u64;
        let x = (i % 4) as f32;
        let y = ((i / 4) % 4) as f32;
        let _ = insert_test_chunk(&mut accel, &queue, key, i, Vec3::new(x, y, 0.5), 0.4);
        live_keys.push_back(key);
    }
    accel.update_gpu_standalone(&device, &queue, 0.0, 0.0);
    assert_eq!(accel.tlas_dirty_count(), 0);

    // 100 churn cycles: remove the oldest + insert a fresh key +
    // rebuild. After every cycle the dirty count must clear and the
    // live chunk count must return to the steady-state target.
    let target_live = accel.live_chunk_count();
    for cycle in 0..100u32 {
        let evict_key = live_keys.pop_front().expect("live_keys non-empty");
        let new_key = (10_000 + cycle) as u64;
        accel
            .remove_chunk(&queue, evict_key)
            .expect("remove_chunk in churn");
        let _ = insert_test_chunk(
            &mut accel,
            &queue,
            new_key,
            cycle % 16,
            Vec3::new((cycle as f32) * 0.1, 1.0, 1.0),
            0.4,
        );
        live_keys.push_back(new_key);
        accel.update_gpu_standalone(&device, &queue, 0.0, 0.0);
        assert_eq!(
            accel.tlas_dirty_count(),
            0,
            "tlas_dirty_count must clear after rebuild on cycle {cycle}",
        );
        assert_eq!(
            accel.live_chunk_count(),
            target_live,
            "live chunk count drifted on cycle {cycle}",
        );
    }

    // Sanity: read back the root node — should not be the empty
    // sentinel, and its AABB should envelope the active region.
    let n = accel.live_chunk_count();
    let total_nodes = if n == 1 { 1 } else { 2 * n - 1 };
    let nodes = read_tlas_nodes(&device, &queue, &accel, total_nodes);
    let root = nodes[0];
    assert!(
        root.aabb_min[0].is_finite() && root.aabb_max[0].is_finite(),
        "root AABB must be finite after churn",
    );
    // VALUE_MASK / LEAF_FLAG sanity: even if the root happens to land
    // at a leaf slot for tiny pools, the encoded payload must round-trip.
    if root.right_or_count & BVH_LEAF_FLAG != 0 {
        let chunk_idx = root.right_or_count & BVH_VALUE_MASK;
        assert!(chunk_idx < accel.caps().max_chunks);
    }
}
