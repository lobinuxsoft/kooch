//! Repro for #356: SDF entity move from the editor must propagate to
//! the slot-resident `leaf_aabbs` buffer the fragment shader binds.
//!
//! The bug surfaced post-#355: dragging a SDF gizmo / editing the
//! Inspector Transform makes some entities disappear. The four
//! candidate causes (refit not propagating AABBs, hash insensitive to
//! moves, slot not flipping, stale `k_max` inflation) all funnel
//! through the same observable symptom: after `kick_auto_if_dirty +
//! poll_swap`, the post-swap slot's `leaf_aabbs[i]` does not match the
//! entity's new world-space AABB.
//!
//! These tests exercise the full `BvhState` lifecycle (the same path
//! `update.rs` walks every frame) without the editor / ECS layer, then
//! read back the resident `leaf_aabbs` buffer and compare each leaf to
//! its expected post-move AABB.

use glam::{Quat, Vec3};
use ome_bvh::{IS_RAYMARCH, LeafAabb, ROLE_RAYMARCH_ADD};

use super::harness::{drive_bvh_to_completion, items_from_leaves, readback_pod, try_acquire_device};
use crate::raymarch::aabb::primitive_aabb;
use crate::raymarch::bvh::BvhState;
use crate::raymarch::instance::{RaymarchPayload, SdfPrimitive, TYPE_SPHERE};

const POSITION_EPS: f32 = 1e-4;

/// AABB tolerance: leaf AABBs round-trip through GPU storage as f32,
/// so bit-equality is fine for axis-aligned sphere primitives. The
/// extra slack absorbs any future float jitter from refit-path
/// arithmetic.
const AABB_EPS: f32 = 1e-4;

/// Fixed 3-sphere scene that maps onto the AC's "≥3 leaves" constraint.
/// Centres deliberately spread across X so a +5 shift on entity 0
/// pushes it well outside the original bounds.
fn three_sphere_scene() -> (Vec<SdfPrimitive>, Vec<LeafAabb>, Vec<RaymarchPayload>) {
    let centres: [Vec3; 3] = [
        Vec3::new(-2.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(2.0, 0.0, 0.0),
    ];
    let radius = 1.0;
    let mut prims = Vec::with_capacity(3);
    let mut leaves = Vec::with_capacity(3);
    let mut payloads = Vec::with_capacity(3);
    for (i, c) in centres.iter().enumerate() {
        let prim = SdfPrimitive {
            position: c.to_array(),
            type_tag: TYPE_SPHERE,
            rotation: Quat::IDENTITY.to_array(),
            scale: [1.0; 3],
            smoothness: 0.0,
            params: [radius, 0.0, 0.0, 0.0],
        };
        let aabb = primitive_aabb(&prim, 0.0);
        leaves.push(LeafAabb {
            aabb_min: aabb.min.to_array(),
            flags: IS_RAYMARCH | ROLE_RAYMARCH_ADD,
            aabb_max: aabb.max.to_array(),
            entity_id: i as u32,
        });
        payloads.push(RaymarchPayload { smoothness: 0.0 });
        prims.push(prim);
    }
    (prims, leaves, payloads)
}

fn assert_aabb_close(actual: &LeafAabb, expected: &LeafAabb, ctx: &str) {
    let am = Vec3::from_array(actual.aabb_min);
    let ex = Vec3::from_array(expected.aabb_min);
    assert!(
        (am - ex).length() < AABB_EPS,
        "{ctx}: aabb_min diverged — actual={:?}, expected={:?}",
        actual.aabb_min,
        expected.aabb_min,
    );
    let am = Vec3::from_array(actual.aabb_max);
    let ex = Vec3::from_array(expected.aabb_max);
    assert!(
        (am - ex).length() < AABB_EPS,
        "{ctx}: aabb_max diverged — actual={:?}, expected={:?}",
        actual.aabb_max,
        expected.aabb_max,
    );
    assert_eq!(
        actual.flags, expected.flags,
        "{ctx}: flags drifted across the kick"
    );
    assert_eq!(
        actual.entity_id, expected.entity_id,
        "{ctx}: entity_id drifted across the kick"
    );
}

/// Build (V1) → mutate one entity's position past every refit
/// threshold (V2) → kick_auto_if_dirty must commit a rebuild and the
/// post-swap slot's `leaf_aabbs[0]` must reflect the new AABB. Other
/// entities stay byte-identical. This is the editor "drag entity by
/// 5 units" path.
#[test]
fn large_move_propagates_to_leaf_aabbs() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("raymarch_bvh::gpu_tests: no GPU adapter — skipping #356 large-move repro");
        return;
    };

    let (prims_v1, leaves_v1, payloads_v1) = three_sphere_scene();
    let items_v1 = items_from_leaves(&leaves_v1);

    let mut state = BvhState::new(&device, &queue, None);
    let kicked = state.kick_auto_if_dirty(
        &device,
        &queue,
        items_v1,
        leaves_v1.clone(),
        payloads_v1.clone(),
        prims_v1.clone(),
        0.25,
        10.0,
    );
    assert!(kicked, "first kick on a fresh BvhState must commit");
    drive_bvh_to_completion(&mut state, &device, &queue);
    assert_eq!(state.current_n(), 3);

    let v1_gpu = readback_pod::<LeafAabb>(&device, &queue, state.current_leaf_aabbs(), 3, "move_propagates::leaf_aabbs_staging");
    for i in 0..3 {
        assert_aabb_close(&v1_gpu[i], &leaves_v1[i], &format!("V1 leaf[{i}]"));
    }

    // V2: shift entity 0 by +5 in X — production AC says "drag the
    // gizmo". Half-extent of the unit sphere is 1.0 → 5.0 / (2 * 1.0)
    // = 250% of the extent → 1 of 3 entities moved = 33% > 10% →
    // kick_auto picks rebuild over refit.
    let mut prims_v2 = prims_v1.clone();
    prims_v2[0].position[0] += 5.0;
    let mut leaves_v2 = leaves_v1.clone();
    let new_aabb = primitive_aabb(&prims_v2[0], 0.0);
    leaves_v2[0].aabb_min = new_aabb.min.to_array();
    leaves_v2[0].aabb_max = new_aabb.max.to_array();
    let items_v2 = items_from_leaves(&leaves_v2);

    let kicked = state.kick_auto_if_dirty(
        &device,
        &queue,
        items_v2,
        leaves_v2.clone(),
        payloads_v1,
        prims_v2.clone(),
        0.25,
        10.0,
    );
    assert!(kicked, "post-move scene hash differs → kick must commit");
    drive_bvh_to_completion(&mut state, &device, &queue);
    assert_eq!(state.current_n(), 3, "rebuild keeps cardinality");

    let v2_gpu = readback_pod::<LeafAabb>(&device, &queue, state.current_leaf_aabbs(), 3, "move_propagates::leaf_aabbs_staging");
    assert_aabb_close(&v2_gpu[0], &leaves_v2[0], "V2 leaf[0] (moved entity)");
    assert_aabb_close(&v2_gpu[1], &leaves_v1[1], "V2 leaf[1] (unchanged)");
    assert_aabb_close(&v2_gpu[2], &leaves_v1[2], "V2 leaf[2] (unchanged)");
}

/// Same shape as the large-move test but the V2 shift is small enough
/// to fall under the `should_refit` thresholds (0.25 × max_dim,
/// `change_threshold_pct = 10.0`). Forces `kick_auto_if_dirty` down
/// the **refit** fast-path. Hypothesis 1 of the issue claims refit
/// fails to propagate AABBs to the slot's `leaf_aabbs` buffer; this
/// test pins that contract directly.
#[test]
fn small_move_through_refit_path_propagates() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("raymarch_bvh::gpu_tests: no GPU adapter — skipping #356 refit-path repro");
        return;
    };

    let (prims_v1, leaves_v1, payloads_v1) = three_sphere_scene();
    let items_v1 = items_from_leaves(&leaves_v1);

    let mut state = BvhState::new(&device, &queue, None);
    state.kick_auto_if_dirty(
        &device,
        &queue,
        items_v1,
        leaves_v1.clone(),
        payloads_v1.clone(),
        prims_v1.clone(),
        0.25,
        10.0,
    );
    drive_bvh_to_completion(&mut state, &device, &queue);
    assert_eq!(state.current_n(), 3);

    // V2: shift entity 0 by 0.1 in X. AABB extent is 2.0 → 0.1 / 2.0
    // = 5% of max-dim, well under 25%. Only one entity moves, but
    // the heuristic counts the percentage of *moved* entities ≥ the
    // ratio — at 5% per-entity displacement, none exceed the ratio,
    // so `should_refit` returns true.
    let mut prims_v2 = prims_v1.clone();
    prims_v2[0].position[0] += 0.1;
    let mut leaves_v2 = leaves_v1.clone();
    let new_aabb = primitive_aabb(&prims_v2[0], 0.0);
    leaves_v2[0].aabb_min = new_aabb.min.to_array();
    leaves_v2[0].aabb_max = new_aabb.max.to_array();
    let items_v2 = items_from_leaves(&leaves_v2);

    let kicked = state.kick_auto_if_dirty(
        &device,
        &queue,
        items_v2,
        leaves_v2.clone(),
        payloads_v1,
        prims_v2.clone(),
        0.25,
        10.0,
    );
    assert!(kicked, "post-move scene hash differs → kick must commit");
    drive_bvh_to_completion(&mut state, &device, &queue);
    assert_eq!(state.current_n(), 3);

    let v2_gpu = readback_pod::<LeafAabb>(&device, &queue, state.current_leaf_aabbs(), 3, "move_propagates::leaf_aabbs_staging");
    assert_aabb_close(&v2_gpu[0], &leaves_v2[0], "V2 leaf[0] (refit path)");
    assert_aabb_close(&v2_gpu[1], &leaves_v1[1], "V2 leaf[1] (refit path)");
    assert_aabb_close(&v2_gpu[2], &leaves_v1[2], "V2 leaf[2] (refit path)");
}

/// Lockstep contract — the slot the renderer binds for primitives
/// must always match the slot's `leaf_aabbs`. Without this, the BVH
/// cull and the SDF eval read different scene states, which is
/// exactly the visibility regression #356 surfaces in the editor.
///
/// The test mutates entity 0's position, kicks `kick_auto_if_dirty`,
/// drives to completion, then reads back BOTH the slot's leaf AABBs
/// AND the slot's primitives buffer. The position embedded in
/// `primitives[0]` must match the moved entity's new position, AND
/// the AABB of `leaf_aabbs[0]` must enclose that position.
#[test]
fn primitives_buffer_stays_in_lockstep_with_leaf_aabbs() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!(
            "raymarch_bvh::gpu_tests: no GPU adapter — skipping #356 lockstep contract repro"
        );
        return;
    };

    let (prims_v1, leaves_v1, payloads_v1) = three_sphere_scene();
    let items_v1 = items_from_leaves(&leaves_v1);

    let mut state = BvhState::new(&device, &queue, None);
    state.kick_auto_if_dirty(
        &device,
        &queue,
        items_v1,
        leaves_v1.clone(),
        payloads_v1.clone(),
        prims_v1.clone(),
        0.25,
        10.0,
    );
    drive_bvh_to_completion(&mut state, &device, &queue);

    let v1_prims = readback_pod::<SdfPrimitive>(&device, &queue, state.current_primitives(), 3, "move_propagates::primitives_staging");
    for i in 0..3 {
        let actual = Vec3::from_array(v1_prims[i].position);
        let expected = Vec3::from_array(prims_v1[i].position);
        assert!(
            (actual - expected).length() < POSITION_EPS,
            "V1 primitive[{i}] position diverged: actual={actual:?}, expected={expected:?}",
        );
    }

    let mut prims_v2 = prims_v1.clone();
    prims_v2[0].position[0] += 5.0;
    let mut leaves_v2 = leaves_v1.clone();
    let new_aabb = primitive_aabb(&prims_v2[0], 0.0);
    leaves_v2[0].aabb_min = new_aabb.min.to_array();
    leaves_v2[0].aabb_max = new_aabb.max.to_array();
    let items_v2 = items_from_leaves(&leaves_v2);

    state.kick_auto_if_dirty(
        &device,
        &queue,
        items_v2,
        leaves_v2.clone(),
        payloads_v1,
        prims_v2.clone(),
        0.25,
        10.0,
    );
    drive_bvh_to_completion(&mut state, &device, &queue);

    let v2_prims = readback_pod::<SdfPrimitive>(&device, &queue, state.current_primitives(), 3, "move_propagates::primitives_staging");
    let v2_leaves = readback_pod::<LeafAabb>(&device, &queue, state.current_leaf_aabbs(), 3, "move_propagates::leaf_aabbs_staging");

    // Lockstep #1: primitives[0] reflects the new position.
    let actual = Vec3::from_array(v2_prims[0].position);
    let expected = Vec3::from_array(prims_v2[0].position);
    assert!(
        (actual - expected).length() < POSITION_EPS,
        "V2 primitive[0] (moved entity) lost the post-edit position: \
         actual={actual:?}, expected={expected:?}",
    );

    // Lockstep #2: leaf_aabbs[0] encloses primitive[0]. With BOTH
    // buffers slot-rotated together, the cull never rejects an
    // entity that the SDF eval would render — the disappearance
    // mode in #356 is impossible by construction.
    for axis in 0..3 {
        let pos = actual.to_array()[axis];
        let lo = v2_leaves[0].aabb_min[axis];
        let hi = v2_leaves[0].aabb_max[axis];
        assert!(
            pos >= lo - POSITION_EPS && pos <= hi + POSITION_EPS,
            "V2 leaf[0] AABB on axis {axis} ({lo}..={hi}) does not enclose \
             primitive[0].position[{axis}] = {pos} — slot lockstep broken",
        );
    }

    // Sanity: untouched entities are still byte-identical (entity_id
    // catches it cleanly even if positions happen to coincide).
    for i in 1..3 {
        let actual = Vec3::from_array(v2_prims[i].position);
        let expected = Vec3::from_array(prims_v1[i].position);
        assert!(
            (actual - expected).length() < POSITION_EPS,
            "V2 primitive[{i}] (unchanged) drifted",
        );
    }
}
