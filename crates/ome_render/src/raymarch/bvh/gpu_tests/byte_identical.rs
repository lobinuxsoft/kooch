//! Cull-vs-cull byte-identical determinism (S9 of PR-4 #115).
//!
//! Two consecutive runs of `eval_scene_bvh` over the same scene + the
//! same sample points must produce bit-identical float output. Proves
//! the per-role accumulator visit order is a function of BVH topology
//! only (never of runtime ray geometry).

use super::harness::{
    default_test_meta, drive_bvh_to_completion, items_from_leaves, random_sphere_scene,
    run_eval_pass, sample_points_grid, try_acquire_device,
};
use crate::raymarch::bvh::BvhState;

/// Eight spheres on a regular grid; same scene rendered twice; the
/// per-sample float output of the second run must be byte-identical
/// to the first. Validates the determinism of the BVH topology
/// (Karras + onesweep — already covered in PR-3) plus the
/// deterministic accumulator ordering inside `eval_scene_bvh`.
#[test]
fn cull_vs_cull_byte_identical_n_8() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("raymarch_bvh::gpu_tests: no GPU adapter — skipping");
        return;
    };
    let (primitives, leaves, payloads) = random_sphere_scene(8, 0xc0ffee01);
    let items = items_from_leaves(&leaves);

    let mut state = BvhState::new(&device, &queue, None);
    state.kick_if_dirty(&device, &queue, items, leaves.clone(), payloads.clone());
    drive_bvh_to_completion(&mut state, &device, &queue);
    assert_eq!(state.current_n(), 8, "build must populate slot");

    let samples = sample_points_grid(512);
    let meta = default_test_meta(&state, primitives.len());

    let run_a = run_eval_pass(
        &device, &queue, &state, &primitives, &leaves, &payloads, &samples, &meta, "cs_main",
    );
    let run_b = run_eval_pass(
        &device, &queue, &state, &primitives, &leaves, &payloads, &samples, &meta, "cs_main",
    );
    assert_eq!(run_a.len(), run_b.len());
    for (i, (a, b)) in run_a.iter().zip(run_b.iter()).enumerate() {
        // bit-exact equality — `assert_eq!` on f32 already does
        // bitwise compare, but explicit `to_bits()` makes the
        // intent unambiguous against future readers.
        assert_eq!(
            a.to_bits(),
            b.to_bits(),
            "sample[{i}] diverged across runs: {a} vs {b}",
        );
    }
}

/// Same property at 1024 leaves — the BVH has multiple internal
/// levels here, and the per-role accumulator visits each leaf in
/// a strictly topology-driven order. Catches any latent
/// non-determinism in stack push ordering or atomic accumulation.
#[test]
fn cull_vs_cull_byte_identical_n_1024() {
    let Some((device, queue)) = try_acquire_device() else { return; };
    let (primitives, leaves, payloads) = random_sphere_scene(1024, 0xfeedface);
    let items = items_from_leaves(&leaves);

    let mut state = BvhState::new(&device, &queue, None);
    state.kick_if_dirty(&device, &queue, items, leaves.clone(), payloads.clone());
    drive_bvh_to_completion(&mut state, &device, &queue);
    assert_eq!(state.current_n(), 1024);

    let samples = sample_points_grid(2048);
    let meta = default_test_meta(&state, primitives.len());

    let run_a = run_eval_pass(
        &device, &queue, &state, &primitives, &leaves, &payloads, &samples, &meta, "cs_main",
    );
    let run_b = run_eval_pass(
        &device, &queue, &state, &primitives, &leaves, &payloads, &samples, &meta, "cs_main",
    );
    for (i, (a, b)) in run_a.iter().zip(run_b.iter()).enumerate() {
        assert_eq!(
            a.to_bits(),
            b.to_bits(),
            "sample[{i}] diverged across runs at N=1024",
        );
    }
}
