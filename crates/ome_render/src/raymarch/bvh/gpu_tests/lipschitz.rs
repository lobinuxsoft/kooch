//! Cull-vs-fullscan Lipschitz-bounded test (S10 of PR-4 #115).
//!
//! Compares `eval_scene_bvh` against a brute-force `eval_scene_
//! fullscan` baseline on the same scene + samples. Restricts the
//! comparison to sample points that fall inside at least one
//! primitive's inflated AABB — outside that region, the BVH
//! short-circuits to `+inf` (no surface) while fullscan returns
//! the distance to the nearest primitive (positive). Both are
//! correct sphere-tracing answers ("step at least this far"), but
//! they are not directly comparable as scalar SDF values.
//!
//! **Bound documented:** for points inside an AABB, `|bvh -
//! fullscan| ≤ k_max + 4·ULP`. Proof sketch: smooth_union at
//! `k → 0` collapses to plain `min`, which IS associative; for
//! `k > 0`, smooth_union's bias decays exponentially past the
//! support radius `k`, so a primitive whose distance > 4·k
//! contributes within ULPs to the result. Test scenes here use
//! `k = 0` for every primitive, so the bound effectively becomes
//! the float-rounding floor.

use super::harness::{
    SamplePoint, default_test_meta, drive_bvh_to_completion, items_from_leaves,
    random_sphere_scene, run_eval_pass, try_acquire_device,
};
use crate::raymarch::bvh::BvhState;

#[test]
fn cull_vs_fullscan_lipschitz_bounded() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("raymarch_bvh::gpu_tests: no GPU adapter — skipping");
        return;
    };
    let (primitives, leaves, payloads) = random_sphere_scene(256, 0xdeadbeef);
    let items = items_from_leaves(&leaves);

    let mut state = BvhState::new(&device, &queue, None);
    state.kick_if_dirty(&device, &queue, items, leaves.clone(), payloads.clone());
    drive_bvh_to_completion(&mut state, &device, &queue);
    assert_eq!(state.current_n(), 256);

    // Sample at every primitive's centre — guaranteed inside its own
    // inflated AABB, which is what the test bound covers. Adding a
    // small radial offset on each axis helps cover both the
    // SDF-positive rim and the strict centre.
    let mut samples: Vec<SamplePoint> = Vec::with_capacity(primitives.len() * 2);
    for prim in &primitives {
        samples.push(SamplePoint {
            pos: [prim.position[0], prim.position[1], prim.position[2], 0.0],
        });
        samples.push(SamplePoint {
            pos: [
                prim.position[0] + 0.1,
                prim.position[1] - 0.05,
                prim.position[2] + 0.07,
                0.0,
            ],
        });
    }

    let meta = default_test_meta(&state, primitives.len());

    let bvh = run_eval_pass(
        &device, &queue, &state, &primitives, &leaves, &payloads, &samples, &meta, "cs_main",
    );
    let full = run_eval_pass(
        &device, &queue, &state, &primitives, &leaves, &payloads, &samples, &meta, "cs_fullscan",
    );

    // Bound: `k_max + 4·ULP` — for k=0 scenes, this is essentially
    // 4 ULPs of the largest accumulator value (~100, the world
    // box edge length). 4 * (100 * f32::EPSILON) ≈ 5e-5. Padded
    // to 1e-4 to absorb the smooth_union(k=1e-5) floor.
    let bound: f32 = 1.0e-4;

    // Restrict comparison to samples inside at least one primitive
    // AABB — outside that region BVH returns +inf and fullscan
    // returns the distance to the nearest primitive; both are
    // valid sphere-tracing outputs but not scalar-comparable.
    let mut compared = 0u32;
    let mut max_diff: f32 = 0.0;
    for (i, sample) in samples.iter().enumerate() {
        let p = glam::Vec3::new(sample.pos[0], sample.pos[1], sample.pos[2]);
        let inside = leaves.iter().any(|l| {
            let lo = glam::Vec3::from_array(l.aabb_min);
            let hi = glam::Vec3::from_array(l.aabb_max);
            p.cmpge(lo).all() && p.cmple(hi).all()
        });
        if !inside {
            continue;
        }
        let diff = (bvh[i] - full[i]).abs();
        max_diff = max_diff.max(diff);
        assert!(
            diff <= bound,
            "sample[{i}] @ {p:?}: bvh={} fullscan={} diff={} > bound={}",
            bvh[i],
            full[i],
            diff,
            bound,
        );
        compared += 1;
    }
    assert!(
        compared >= 64,
        "test sample distribution did not produce enough in-AABB points: {compared}",
    );
    eprintln!(
        "cull_vs_fullscan: {compared} samples compared, max |diff| = {max_diff} (bound {bound})",
    );
}
