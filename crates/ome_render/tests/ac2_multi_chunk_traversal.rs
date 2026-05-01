//! AC2 — multi-chunk traversal correctness.
//!
//! Three spatially adjacent chunks loaded into `OmeAccel`. Sample the
//! pool-driven `eval_scene_bvh` at 1 000 random points and assert the
//! GPU output matches a CPU ground truth that **folds scene-wide**
//! over every primitive in every chunk — NOT
//! `chunks.iter().map(eval_chunk).reduce()` (the per-chunk reduce
//! would lose cross-chunk smooth-blend bleed and is the canonical
//! pitfall the issue body documents).
//!
//! Tolerance: `1e-5` per the issue body.

mod common;

use bytemuck::Pod;
use common::{
    EvalPipeline, SamplePoint, SmokePrimitive, dispatch_eval_pass, sphere_leaf,
    try_acquire_device,
};
use glam::Vec3;
use ome_bvh::{AccelCaps, ChunkInsert, OmeAccel};

const CHUNK_SIDE: f32 = 6.0;
const N_PRIMS_PER_CHUNK: usize = 4;

/// Build chunk `cx`'s primitive set: `N_PRIMS_PER_CHUNK` spheres laid
/// out in a 2×2 grid inside the chunk's xy-quadrant. Each chunk
/// occupies `[cx * CHUNK_SIDE, (cx+1) * CHUNK_SIDE] × [-2, 2]^2`.
fn build_chunk_scene(cx: i32) -> (Vec<SmokePrimitive>, Vec<ome_bvh::LeafAabb>) {
    let chunk_origin_x = (cx as f32) * CHUNK_SIDE;
    let mut prims = Vec::new();
    let mut leaves = Vec::new();
    for i in 0..N_PRIMS_PER_CHUNK {
        let dx = (i as f32 % 2.0) * 2.0 + 1.0;
        let dy = ((i as f32 / 2.0).floor()) * 2.0 - 1.0;
        let centre = [chunk_origin_x + dx, dy, 0.0];
        prims.push(SmokePrimitive::sphere(centre, 0.6));
        // Bias `cx` into a non-negative range before mixing with `i` —
        // raw `cx as u32` wraps for negative chunks and overflows the
        // multiply.
        let cx_id = (cx + 8) as u32;
        leaves.push(sphere_leaf(centre, 0.6, cx_id * 100 + i as u32));
    }
    (prims, leaves)
}

/// CPU mirror of `eval_scene_bvh`'s per-role fold, scene-wide. Walks
/// every primitive across every chunk with no per-chunk reset of the
/// accumulators — that's the correctness invariant the test exists to
/// pin (architecture note 2 of #360). Mirrors the shader's
/// conditional final combine on `has_intersects` / `has_subs` so the
/// `±1e6` identities don't bleed `mix(a, b, t)` precision into the
/// comparison.
fn eval_scene_cpu(
    p: Vec3,
    chunks: &[(Vec<SmokePrimitive>, Vec<ome_bvh::LeafAabb>)],
    k_int_global: f32,
    k_sub_global: f32,
) -> f32 {
    let mut acc_add = 1.0e6_f32;
    let mut acc_int = -1.0e6_f32;
    let mut acc_sub = 1.0e6_f32;
    let mut has_intersects = false;
    let mut has_subs = false;
    for (prims, leaves) in chunks {
        for (prim, leaf) in prims.iter().zip(leaves.iter()) {
            if (leaf.flags & ome_bvh::IS_RAYMARCH) == 0 {
                continue;
            }
            let role = leaf.flags & 0x3;
            if role == 1 {
                has_intersects = true;
            } else if role == 2 {
                has_subs = true;
            }
            // Brute-force ground truth: evaluate every primitive
            // unconditionally. The shader's BVH descend prunes by
            // distance-to-AABB (`sdf_aabb(p, node) > acc_add`), which
            // is sound under the codebase's smoothness-inflated AABB
            // convention — so the GPU result coincides with this fold
            // within float tolerance, but the test stays a *correct*
            // ground truth and is no longer co-buggy with the shader
            // (the legacy `if !inside { continue; }` was a point-query
            // mirror of the shader bug fixed in this PR).
            let _ = leaf;
            let d = sdf_sphere_world(p, prim);
            let k = prim.smoothness.max(1e-5);
            match role {
                1 => acc_int = smooth_intersection(acc_int, d, k),
                2 => acc_sub = smooth_union(acc_sub, d, k),
                _ => acc_add = smooth_union(acc_add, d, k),
            }
        }
    }
    let mut result = acc_add;
    if has_intersects {
        result = smooth_intersection(result, acc_int, k_int_global.max(1e-5));
    }
    if has_subs {
        result = smooth_subtraction(result, acc_sub, k_sub_global.max(1e-5));
    }
    result
}

fn sdf_sphere_world(p: Vec3, prim: &SmokePrimitive) -> f32 {
    let local = p - Vec3::from_array(prim.position);
    local.length() - prim.params[0]
}

fn smooth_union(d1: f32, d2: f32, k: f32) -> f32 {
    let h = (0.5 + 0.5 * (d2 - d1) / k).clamp(0.0, 1.0);
    d2 * (1.0 - h) + d1 * h - k * h * (1.0 - h)
}

fn smooth_intersection(d1: f32, d2: f32, k: f32) -> f32 {
    let h = (0.5 - 0.5 * (d2 - d1) / k).clamp(0.0, 1.0);
    d2 * (1.0 - h) + d1 * h + k * h * (1.0 - h)
}

fn smooth_subtraction(d1: f32, d2: f32, k: f32) -> f32 {
    let h = (0.5 - 0.5 * (d2 + d1) / k).clamp(0.0, 1.0);
    d1 * (1.0 - h) + (-d2) * h + k * h * (1.0 - h)
}

fn lcg(state: &mut u32) -> f32 {
    *state = state.wrapping_mul(1664525).wrapping_add(1013904223);
    (*state >> 8) as f32 / (1u32 << 24) as f32
}

#[test]
fn ac2_multi_chunk_traversal_matches_scene_wide_fold() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("AC2: no adapter — skipping");
        return;
    };

    // 3 spatially adjacent chunks at chunk_x = -1, 0, 1.
    let chunks: Vec<(Vec<SmokePrimitive>, Vec<ome_bvh::LeafAabb>)> =
        (-1..=1).map(build_chunk_scene).collect();

    let mut accel = OmeAccel::new(
        &device,
        AccelCaps::default(),
        std::mem::size_of::<SmokePrimitive>() as u32,
    )
    .unwrap();

    for (chunk_idx, (prims, leaves)) in chunks.iter().enumerate() {
        let prim_bytes: &[u8] = bytemuck::cast_slice::<SmokePrimitive, u8>(prims);
        accel
            .insert_chunk(
                &queue,
                ChunkInsert {
                    key: chunk_idx as u64,
                    leaf_aabbs: leaves,
                    primitives_bytes: prim_bytes,
                    max_smoothness_radius: 0.0,
                },
            )
            .unwrap();
    }
    accel.update_gpu_standalone(&device, &queue, 0.0, 0.0);

    // 1 000 sample points distributed over the scene volume — covers
    // both inside-leaf and outside-leaf code paths and hits chunk
    // boundaries.
    let mut state = 0xa2_b00b_u32;
    let samples: Vec<SamplePoint> = (0..1_000u32)
        .map(|_| {
            // Span all three chunks plus a margin: x ∈ [-CHUNK_SIDE, 2*CHUNK_SIDE].
            let x = (lcg(&mut state) - 1.0 / 3.0) * 3.0 * CHUNK_SIDE;
            let y = (lcg(&mut state) - 0.5) * 5.0;
            let z = (lcg(&mut state) - 0.5) * 5.0;
            SamplePoint::at(x, y, z)
        })
        .collect();

    let pipeline = EvalPipeline::new(&device);
    let gpu = dispatch_eval_pass(&device, &queue, &pipeline, &accel, &samples);

    let mut max_diff = 0.0_f32;
    let mut worst_idx = 0;
    for (i, s) in samples.iter().enumerate() {
        let p = Vec3::new(s.p[0], s.p[1], s.p[2]);
        let cpu = eval_scene_cpu(p, &chunks, 0.0, 0.0);
        let g = gpu[i];
        let diff = (g - cpu).abs();
        if diff > max_diff {
            max_diff = diff;
            worst_idx = i;
        }
    }
    let p = samples[worst_idx];
    assert!(
        max_diff < 1e-5,
        "AC2: max |gpu - cpu| = {max_diff} at sample {worst_idx} = ({}, {}, {})",
        p.p[0], p.p[1], p.p[2],
    );
}

/// Empty chunks + populated chunks must compose: an evicted chunk in
/// the middle of the pool does not corrupt traversal of its
/// neighbours. Pins the no-stale-data invariant of `tlas::rebuild`.
#[test]
fn ac2_evicted_middle_chunk_does_not_pollute_traversal() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("AC2: no adapter — skipping");
        return;
    };
    let chunks: Vec<(Vec<SmokePrimitive>, Vec<ome_bvh::LeafAabb>)> =
        (-1..=1).map(build_chunk_scene).collect();
    let mut accel = OmeAccel::new(
        &device,
        AccelCaps::default(),
        std::mem::size_of::<SmokePrimitive>() as u32,
    )
    .unwrap();
    for (chunk_idx, (prims, leaves)) in chunks.iter().enumerate() {
        let prim_bytes: &[u8] = pod_bytes(prims);
        accel
            .insert_chunk(
                &queue,
                ChunkInsert {
                    key: chunk_idx as u64,
                    leaf_aabbs: leaves,
                    primitives_bytes: prim_bytes,
                    max_smoothness_radius: 0.0,
                },
            )
            .unwrap();
    }
    // Evict middle chunk + rebuild TLAS.
    accel.remove_chunk(&queue, 1).unwrap();
    accel.update_gpu_standalone(&device, &queue, 0.0, 0.0);

    // Sample only inside the surviving chunks — must produce non-
    // identity values.
    let samples: Vec<SamplePoint> = vec![
        SamplePoint::at(-CHUNK_SIDE + 1.0, 0.0, 0.0),
        SamplePoint::at(CHUNK_SIDE + 1.0, 0.0, 0.0),
    ];
    let pipeline = EvalPipeline::new(&device);
    let gpu = dispatch_eval_pass(&device, &queue, &pipeline, &accel, &samples);

    let surviving: Vec<_> = chunks
        .into_iter()
        .enumerate()
        .filter(|(idx, _)| *idx != 1)
        .map(|(_, c)| c)
        .collect();
    for (i, s) in samples.iter().enumerate() {
        let p = Vec3::new(s.p[0], s.p[1], s.p[2]);
        let cpu = eval_scene_cpu(p, &surviving, 0.0, 0.0);
        let diff = (gpu[i] - cpu).abs();
        assert!(
            diff < 1e-5,
            "post-eviction sample {i}: gpu={} cpu={cpu} diff={diff}",
            gpu[i],
        );
    }
}

fn pod_bytes<T: Pod>(slice: &[T]) -> &[u8] {
    bytemuck::cast_slice::<T, u8>(slice)
}
