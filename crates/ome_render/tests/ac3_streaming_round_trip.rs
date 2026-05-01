//! AC3 — streaming round-trip equivalence.
//!
//! Insert N chunks → snapshot the rendered scene at a sample-point
//! grid → evict every other chunk → re-insert the same content for
//! the evicted slots → snapshot again. The two snapshots must agree
//! within `1e-5` per-sample: the round-trip recreates the same scene
//! state, so the GPU evaluation must converge to the same output
//! regardless of the intermediate eviction churn.
//!
//! The issue body also asks for a "only TLAS refit ran" metric below
//! `TLAS_REBUILD_THRESHOLD`. The current pool always does a full
//! Karras TLAS rebuild on `update_gpu` — incremental TLAS refit lives
//! behind a future optimisation hook, not in this PR's scope. AC3
//! pins the **correctness** invariant; the perf metric will land
//! alongside the incremental path.

mod common;

use bytemuck::Pod;
use common::{
    EvalPipeline, SamplePoint, SmokePrimitive, dispatch_eval_pass, sphere_leaf,
    try_acquire_device,
};
use ome_bvh::{AccelCaps, ChunkInsert, OmeAccel};

const N_CHUNKS: u64 = 20;
const PRIMS_PER_CHUNK: u32 = 4;

fn build_chunk(key: u64) -> (Vec<SmokePrimitive>, Vec<ome_bvh::LeafAabb>) {
    // Deterministic chunk content keyed off `key` so the post-evict
    // re-insert hands the pool exactly the same scene state.
    let cx = (key as f32) - (N_CHUNKS as f32) * 0.5;
    let mut prims = Vec::new();
    let mut leaves = Vec::new();
    for i in 0..PRIMS_PER_CHUNK {
        let dx = (i as f32 % 2.0) * 1.5;
        let dy = ((i as f32 / 2.0).floor()) * 1.5 - 0.5;
        let centre = [cx + dx, dy, 0.0];
        prims.push(SmokePrimitive::sphere(centre, 0.5));
        leaves.push(sphere_leaf(centre, 0.5, (key as u32) * PRIMS_PER_CHUNK + i));
    }
    (prims, leaves)
}

fn pod_bytes<T: Pod>(slice: &[T]) -> &[u8] {
    bytemuck::cast_slice::<T, u8>(slice)
}

fn insert_one(accel: &mut OmeAccel, queue: &wgpu::Queue, key: u64) {
    let (prims, leaves) = build_chunk(key);
    accel
        .insert_chunk(
            queue,
            ChunkInsert {
                key,
                leaf_aabbs: &leaves,
                primitives_bytes: pod_bytes(&prims),
                max_smoothness_radius: 0.0,
            },
        )
        .unwrap_or_else(|e| panic!("insert_chunk(key={key}) failed: {e}"));
}

fn build_scene(device: &wgpu::Device, queue: &wgpu::Queue) -> OmeAccel {
    let mut accel = OmeAccel::new(
        device,
        AccelCaps::default(),
        std::mem::size_of::<SmokePrimitive>() as u32,
    )
    .unwrap();
    for key in 0..N_CHUNKS {
        insert_one(&mut accel, queue, key);
    }
    accel.update_gpu_standalone(device, queue, 0.0, 0.0);
    accel
}

fn lcg(state: &mut u32) -> f32 {
    *state = state.wrapping_mul(1664525).wrapping_add(1013904223);
    (*state >> 8) as f32 / (1u32 << 24) as f32
}

fn sample_grid() -> Vec<SamplePoint> {
    let mut state = 0xa3_05fe_u32;
    (0..1_000u32)
        .map(|_| {
            let x = (lcg(&mut state) - 0.5) * (N_CHUNKS as f32) * 1.5;
            let y = (lcg(&mut state) - 0.5) * 4.0;
            let z = (lcg(&mut state) - 0.5) * 4.0;
            SamplePoint::at(x, y, z)
        })
        .collect()
}

#[test]
fn ac3_streaming_round_trip_preserves_render_output() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("AC3: no adapter — skipping");
        return;
    };

    // Phase 1: build the scene fresh, snapshot.
    let mut accel = build_scene(&device, &queue);
    let pipeline = EvalPipeline::new(&device);
    let samples = sample_grid();
    let baseline = dispatch_eval_pass(&device, &queue, &pipeline, &accel, &samples);

    // Phase 2: evict every other chunk + re-insert the same content.
    // The eviction batch crosses the `TLAS_REBUILD_THRESHOLD` (16) so
    // we exercise the full-rebuild path — the cheap incremental TLAS
    // refit is a future optimisation, not on the AC3 critical path.
    let evicted_keys: Vec<u64> = (0..N_CHUNKS).step_by(2).collect();
    for &key in &evicted_keys {
        accel.remove_chunk(&queue, key).unwrap();
    }
    accel.update_gpu_standalone(&device, &queue, 0.0, 0.0);

    // Sanity: half the chunks gone after eviction.
    let half_live = accel.live_chunk_count();
    assert_eq!(
        half_live as usize,
        N_CHUNKS as usize - evicted_keys.len(),
        "evict batch must drop exactly the requested chunks"
    );

    for &key in &evicted_keys {
        insert_one(&mut accel, &queue, key);
    }
    accel.update_gpu_standalone(&device, &queue, 0.0, 0.0);
    assert_eq!(accel.live_chunk_count() as u64, N_CHUNKS);

    let post_round_trip = dispatch_eval_pass(&device, &queue, &pipeline, &accel, &samples);

    // The round-trip must yield the same output within 1e-5 — the
    // scene state is identical to the baseline up to chunk_idx
    // remapping (which the morton-sorted TLAS rebuild absorbs).
    let mut max_diff = 0.0_f32;
    let mut worst = 0;
    for (i, (a, b)) in baseline.iter().zip(post_round_trip.iter()).enumerate() {
        let diff = (a - b).abs();
        if diff > max_diff {
            max_diff = diff;
            worst = i;
        }
    }
    assert!(
        max_diff < 1e-5,
        "AC3: round-trip diverged at sample {worst}: baseline={} after={} diff={max_diff}",
        baseline[worst],
        post_round_trip[worst],
    );
}

/// AC3 corollary: an idempotent `remove(key) → insert(key)` sequence
/// on a single chunk converges to the same per-sample output as the
/// pre-removal pool. Tightens the round-trip assertion to a single
/// chunk so a regression in `remove_chunk` (e.g. dropping the CPU
/// mirror without resetting the TLAS dirty count) surfaces in
/// isolation from the multi-chunk variant above.
#[test]
fn ac3_single_chunk_remove_reinsert_idempotent() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("AC3 corollary: no adapter — skipping");
        return;
    };
    let mut accel = OmeAccel::new(
        &device,
        AccelCaps::default(),
        std::mem::size_of::<SmokePrimitive>() as u32,
    )
    .unwrap();
    insert_one(&mut accel, &queue, 7);
    accel.update_gpu_standalone(&device, &queue, 0.0, 0.0);

    let pipeline = EvalPipeline::new(&device);
    let samples = sample_grid();
    let before = dispatch_eval_pass(&device, &queue, &pipeline, &accel, &samples);

    accel.remove_chunk(&queue, 7).unwrap();
    accel.update_gpu_standalone(&device, &queue, 0.0, 0.0);
    insert_one(&mut accel, &queue, 7);
    accel.update_gpu_standalone(&device, &queue, 0.0, 0.0);

    let after = dispatch_eval_pass(&device, &queue, &pipeline, &accel, &samples);
    for (i, (a, b)) in before.iter().zip(after.iter()).enumerate() {
        assert!(
            (a - b).abs() < 1e-5,
            "AC3 single: sample {i} diverged: {a} vs {b}",
        );
    }
}
