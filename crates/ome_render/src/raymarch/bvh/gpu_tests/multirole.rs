//! Multi-role BVH cull regression — pins `eval_scene_bvh`'s prune to
//! be order-independent across mixed ADD/SUB scenes (#354).
//!
//! PR #352 used `min(add_acc, sub_acc)` as the prune bound, which
//! prematurely skipped subtrees that contained SUB leaves whenever
//! any ADD leaf had already been visited. The DFS visit order shifts
//! with the ray direction in production, so the bug surfaced as
//! geometry that appeared/disappeared with the camera angle in the
//! editor's HierarchyTest scene. The fix changes the bound to
//! `max(add_acc, sub_acc)` — a leaf of role R is only safe to skip
//! when `d_aabb > acc_R`, so the conservative multi-role bound is the
//! larger of the two accumulators.
//!
//! This test rebuilds a deterministic 12-primitive scene with mixed
//! ADD and SUB roles, runs both the BVH-driven kernel and the brute-
//! force fullscan baseline at a 12³ sample grid (1728 points spread
//! across the scene volume), and asserts they agree at every point
//! whose nearest primitive is within `4·k_max` (where smooth-union's
//! tail still matters). With the buggy `min` prune the BVH path
//! returns visibly higher distances at points where SUB primitives
//! have been pruned out — fullscan visits every leaf so it sees the
//! true min — and the diff exceeds the bound.

use glam::Quat;
use ome_bvh::{IS_RAYMARCH, LeafAabb, ROLE_RAYMARCH_ADD, ROLE_RAYMARCH_SUB};

use crate::raymarch::aabb::primitive_aabb;
use crate::raymarch::bvh::BvhState;
use crate::raymarch::instance::{RaymarchPayload, SceneMeta, SdfPrimitive, TYPE_BOX, TYPE_SPHERE};

use super::harness::{
    SamplePoint, drive_bvh_to_completion, items_from_leaves, run_eval_pass, try_acquire_device,
};

#[derive(Copy, Clone)]
struct Spec {
    position: [f32; 3],
    type_tag: u32,
    /// Sphere radius OR box half-extent (uniform).
    size: f32,
    role: u32,
}

const SCENE: &[Spec] = &[
    // ADD primitives — a small mountain-like cluster.
    Spec { position: [0.0, 0.0, 0.0], type_tag: TYPE_SPHERE, size: 1.5, role: ROLE_RAYMARCH_ADD },
    Spec { position: [-2.5, 0.0, 0.0], type_tag: TYPE_SPHERE, size: 1.0, role: ROLE_RAYMARCH_ADD },
    Spec { position: [2.5, 0.0, 0.0], type_tag: TYPE_SPHERE, size: 1.0, role: ROLE_RAYMARCH_ADD },
    Spec { position: [0.0, 2.5, 0.0], type_tag: TYPE_SPHERE, size: 1.2, role: ROLE_RAYMARCH_ADD },
    Spec { position: [0.0, -2.5, 0.0], type_tag: TYPE_BOX, size: 1.0, role: ROLE_RAYMARCH_ADD },
    Spec { position: [0.0, 0.0, 2.5], type_tag: TYPE_BOX, size: 1.0, role: ROLE_RAYMARCH_ADD },
    Spec { position: [0.0, 0.0, -2.5], type_tag: TYPE_BOX, size: 0.8, role: ROLE_RAYMARCH_ADD },
    Spec { position: [3.0, 3.0, 0.0], type_tag: TYPE_SPHERE, size: 0.6, role: ROLE_RAYMARCH_ADD },
    // SUB primitives — carve holes through the cluster from
    // off-centre angles. Their AABBs span both the ADD cluster and
    // empty space so the cull's prune behaviour matters here.
    Spec { position: [1.0, 0.5, 0.0], type_tag: TYPE_SPHERE, size: 0.6, role: ROLE_RAYMARCH_SUB },
    Spec { position: [-1.0, 0.5, 0.0], type_tag: TYPE_SPHERE, size: 0.6, role: ROLE_RAYMARCH_SUB },
    Spec { position: [0.0, 1.5, 1.0], type_tag: TYPE_SPHERE, size: 0.5, role: ROLE_RAYMARCH_SUB },
    Spec { position: [0.0, 1.5, -1.0], type_tag: TYPE_SPHERE, size: 0.5, role: ROLE_RAYMARCH_SUB },
];

fn build_scene() -> (Vec<SdfPrimitive>, Vec<LeafAabb>, Vec<RaymarchPayload>) {
    let mut prims = Vec::with_capacity(SCENE.len());
    let mut leaves = Vec::with_capacity(SCENE.len());
    let mut payloads = Vec::with_capacity(SCENE.len());
    for (i, spec) in SCENE.iter().enumerate() {
        let params = match spec.type_tag {
            TYPE_SPHERE => [spec.size, 0.0, 0.0, 0.0],
            TYPE_BOX => [spec.size, spec.size, spec.size, 0.0],
            _ => [0.0; 4],
        };
        let prim = SdfPrimitive {
            position: spec.position,
            type_tag: spec.type_tag,
            rotation: Quat::IDENTITY.to_array(),
            scale: [1.0; 3],
            smoothness: 0.0,
            params,
        };
        let aabb = primitive_aabb(&prim, 0.0);
        leaves.push(LeafAabb {
            aabb_min: aabb.min.to_array(),
            flags: IS_RAYMARCH | spec.role,
            aabb_max: aabb.max.to_array(),
            entity_id: i as u32,
        });
        // k=0 keeps the smooth-blend tail negligible — the only thing
        // we want to measure is the prune behaviour, not the smoothness
        // floor.
        payloads.push(RaymarchPayload { smoothness: 0.0 });
        prims.push(prim);
    }
    (prims, leaves, payloads)
}

fn sample_grid(side: u32, half_extent: f32) -> Vec<SamplePoint> {
    let step = (2.0 * half_extent) / (side as f32 - 1.0).max(1.0);
    let mut out = Vec::with_capacity((side * side * side) as usize);
    for ix in 0..side {
        for iy in 0..side {
            for iz in 0..side {
                let x = -half_extent + ix as f32 * step;
                let y = -half_extent + iy as f32 * step;
                let z = -half_extent + iz as f32 * step;
                out.push(SamplePoint { pos: [x, y, z, 0.0] });
            }
        }
    }
    out
}

fn meta_for(state: &BvhState, primitive_count: usize) -> SceneMeta {
    SceneMeta {
        primitive_count: primitive_count as u32,
        bvh_n: state.current_n(),
        skip_internal_sky: 0,
        has_intersects: 0,
        has_subs: 1,
        k_int_scene: 0.0,
        k_sub_scene: 0.0,
        _pad0: 0,
        sky_top: [0.0; 4],
        sky_bottom: [0.0; 4],
    }
}

#[test]
fn bvh_cull_matches_fullscan_for_mixed_add_sub_scene() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("raymarch_bvh::gpu_tests: no GPU adapter — skipping");
        return;
    };

    let (primitives, leaves, payloads) = build_scene();
    let items = items_from_leaves(&leaves);

    let mut state = BvhState::new(&device, &queue, None);
    state.kick_if_dirty(&device, &queue, items, leaves.clone(), payloads.clone(), primitives.clone());
    drive_bvh_to_completion(&mut state, &device, &queue);
    assert_eq!(state.current_n(), SCENE.len() as u32);

    let samples = sample_grid(12, 4.0);
    let meta = meta_for(&state, primitives.len());

    let bvh = run_eval_pass(
        &device, &queue, &state, &primitives, &leaves, &payloads, &samples, &meta, "cs_main",
    );
    let full = run_eval_pass(
        &device, &queue, &state, &primitives, &leaves, &payloads, &samples, &meta, "cs_fullscan",
    );

    // k=0 scene → bound is just float-rounding noise. Pad to 1e-3 to
    // absorb the smooth_union(k=1e-5) clamp the production shader uses.
    let bound: f32 = 1.0e-3;

    let mut compared = 0u32;
    let mut max_diff: f32 = 0.0;
    let mut worst_idx: usize = 0;
    for i in 0..samples.len() {
        // Compare every sample whose true SDF lands inside the
        // "interesting" range — the band where the cull regression
        // would surface. Outside primitive support both kernels
        // agree trivially (everything returns the union identity).
        if full[i].abs() > 5.0 {
            continue;
        }
        let diff = (bvh[i] - full[i]).abs();
        if diff > max_diff {
            max_diff = diff;
            worst_idx = i;
        }
        compared += 1;
    }
    assert!(
        compared >= 64,
        "scene didn't produce enough samples in primitive-support band: {compared}",
    );
    let p = samples[worst_idx].pos;
    assert!(
        max_diff <= bound,
        "BVH cull diverges from fullscan baseline at sample[{worst_idx}] \
         pos=({:.3}, {:.3}, {:.3}): bvh={} fullscan={} diff={} > bound={}",
        p[0], p[1], p[2], bvh[worst_idx], full[worst_idx], max_diff, bound,
    );
    eprintln!(
        "multirole_cull: {compared} samples compared, max |diff| = {max_diff} (bound {bound})",
    );
}
