use crate::aabb::Aabb;

use glam::Vec3;

use super::helpers::{aabb_at, random_items, run_pair};

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
