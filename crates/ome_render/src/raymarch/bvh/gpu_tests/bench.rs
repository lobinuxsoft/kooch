//! Wall-clock perf scaling bench (S11 of PR-4 #115). NOT a CI gate —
//! every test here is `#[ignore]`-gated. Run manually with:
//!
//! ```text
//!     cargo test -p ome_render --lib bench_scaling \
//!        -- --ignored --nocapture
//! ```
//!
//! Reports the average per-pass time of `cs_main` (BVH-driven) vs
//! `cs_fullscan` (brute-force baseline) at three scene sizes. Speedup
//! grows with scene size — small N is dominated by the BVH stack-walk
//! overhead, large N by per-leaf work the cull eliminates.

use super::harness::{
    SamplePoint, default_test_meta, drive_bvh_to_completion, items_from_leaves,
    random_sphere_scene, run_eval_pass, sample_points_grid, try_acquire_device,
};
use crate::raymarch::bvh::BvhState;

fn bench_pair(n: u32, iters: u32) {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("bench skipped: no GPU adapter");
        return;
    };
    let (primitives, leaves) = random_sphere_scene(n, 0xb_e_e_f);
    let items = items_from_leaves(&leaves);

    let mut state = BvhState::new(&device, &queue, None);
    state.kick_if_dirty(&device, &queue, items, leaves.clone());
    drive_bvh_to_completion(&mut state, &device, &queue);
    assert_eq!(state.current_n(), n);

    // Sample at every primitive centre + 8 grid points per scene
    // to cover both in-AABB and empty regions.
    let mut samples: Vec<SamplePoint> = primitives
        .iter()
        .map(|p| SamplePoint { pos: [p.position[0], p.position[1], p.position[2], 0.0] })
        .collect();
    samples.extend(sample_points_grid(2048));

    let meta = default_test_meta(&state, primitives.len());

    // Warmup (1 pass each) to let drivers compile + cache.
    let _ = run_eval_pass(
        &device, &queue, &state, &primitives, &leaves, &samples, &meta, "cs_main",
    );
    let _ = run_eval_pass(
        &device, &queue, &state, &primitives, &leaves, &samples, &meta, "cs_fullscan",
    );

    let mut bvh_total = std::time::Duration::ZERO;
    let mut full_total = std::time::Duration::ZERO;
    for _ in 0..iters {
        let t0 = std::time::Instant::now();
        let _ = run_eval_pass(
            &device, &queue, &state, &primitives, &leaves, &samples, &meta, "cs_main",
        );
        bvh_total += t0.elapsed();

        let t0 = std::time::Instant::now();
        let _ = run_eval_pass(
            &device, &queue, &state, &primitives, &leaves, &samples, &meta, "cs_fullscan",
        );
        full_total += t0.elapsed();
    }
    let bvh_avg = bvh_total / iters;
    let full_avg = full_total / iters;
    let ratio = full_avg.as_secs_f64() / bvh_avg.as_secs_f64();
    eprintln!(
        "[N={n} samples={} iters={iters}]  bvh: {:>9.2?} avg | fullscan: {:>9.2?} avg | speedup: {:.2}×",
        samples.len(),
        bvh_avg,
        full_avg,
        ratio,
    );
}

#[test]
#[ignore = "bench (run with --ignored --nocapture)"]
fn bench_scaling_1k() {
    bench_pair(1024, 5);
}

#[test]
#[ignore = "bench (run with --ignored --nocapture)"]
fn bench_scaling_10k() {
    bench_pair(10_240, 5);
}

#[test]
#[ignore = "bench (run with --ignored --nocapture)"]
fn bench_scaling_65k() {
    bench_pair(65_000, 3);
}
