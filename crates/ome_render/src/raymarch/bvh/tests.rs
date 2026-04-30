//! CPU-only unit tests for [`super::BvhState`] + the shared per-role
//! smoothness reducer. Production tests against a live `wgpu::Device`
//! live in `tests/pool_eval_smoke.rs` (PR-1) and the AC1 / AC6
//! regression tests under `gpu_tests/` (PR-2).

use super::state::reduce_per_role_smoothness;
use crate::raymarch::instance::{SdfPrimitive, TYPE_SPHERE};
use glam::Quat;
use ome_bvh::{
    IS_COLLIDER, IS_RAYMARCH, LeafAabb, ROLE_RAYMARCH_ADD, ROLE_RAYMARCH_INT, ROLE_RAYMARCH_SUB,
};

fn leaf(role_bits: u32, entity_id: u32) -> LeafAabb {
    LeafAabb {
        aabb_min: [0.0; 3],
        flags: IS_RAYMARCH | role_bits,
        aabb_max: [1.0; 3],
        entity_id,
    }
}

fn prim(smoothness: f32) -> SdfPrimitive {
    SdfPrimitive {
        position: [0.0; 3],
        type_tag: TYPE_SPHERE,
        rotation: Quat::IDENTITY.to_array(),
        scale: [1.0; 3],
        smoothness,
        params: [1.0, 0.0, 0.0, 0.0],
    }
}

#[test]
fn reduce_per_role_picks_max_for_each_role_independently() {
    let leaves = [
        leaf(ROLE_RAYMARCH_ADD, 0),
        leaf(ROLE_RAYMARCH_ADD, 1),
        leaf(ROLE_RAYMARCH_INT, 2),
        leaf(ROLE_RAYMARCH_SUB, 3),
    ];
    let prims = [prim(0.10), prim(0.40), prim(0.25), prim(0.55)];
    let (k_int, k_sub, envelope) = reduce_per_role_smoothness(&leaves, &prims);
    assert!((k_int - 0.25).abs() < 1e-6);
    assert!((k_sub - 0.55).abs() < 1e-6);
    // Envelope is the scene-wide max across all three roles — drives
    // the chunk's `max_smoothness_radius` AABB inflation.
    assert!((envelope - 0.55).abs() < 1e-6);
}

#[test]
fn reduce_per_role_skips_non_raymarch_leaves() {
    let leaves = [
        LeafAabb {
            aabb_min: [0.0; 3],
            flags: IS_COLLIDER, // collider only — must NOT contribute
            aabb_max: [1.0; 3],
            entity_id: 0,
        },
        leaf(ROLE_RAYMARCH_INT, 1),
    ];
    let prims = [prim(0.99), prim(0.30)];
    let (k_int, k_sub, envelope) = reduce_per_role_smoothness(&leaves, &prims);
    // The collider's primitive smoothness is ignored; the int role's
    // 0.30 wins both `k_int_global` and the envelope.
    assert!((k_int - 0.30).abs() < 1e-6);
    assert_eq!(k_sub, 0.0);
    assert!((envelope - 0.30).abs() < 1e-6);
}

#[test]
fn reduce_per_role_returns_zero_for_empty_scene() {
    let (k_int, k_sub, envelope) = reduce_per_role_smoothness(&[], &[]);
    assert_eq!(k_int, 0.0);
    assert_eq!(k_sub, 0.0);
    assert_eq!(envelope, 0.0);
}
