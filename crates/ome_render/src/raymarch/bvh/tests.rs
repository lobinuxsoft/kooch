//! CPU-only unit tests for [`super::BvhState::hash_scene`]. The hash
//! drives `kick_if_dirty`'s short-circuit on an unchanged scene, so any
//! field that participates in the rendered output must change the hash
//! when it changes — otherwise the BVH stays stale across edits.

use super::BvhState;
use crate::raymarch::instance::{RaymarchPayload, SdfPrimitive, TYPE_SPHERE};
use glam::{Quat, Vec3};
use ome_bvh::{Aabb, IS_RAYMARCH, LeafAabb, ROLE_RAYMARCH_ADD, ROLE_RAYMARCH_INT};

fn dummy_leaf(role_bits: u32, entity_id: u32) -> LeafAabb {
    LeafAabb {
        aabb_min: [0.0; 3],
        flags: IS_RAYMARCH | role_bits,
        aabb_max: [1.0; 3],
        entity_id,
    }
}

fn dummy_payload(smoothness: f32) -> RaymarchPayload {
    RaymarchPayload { smoothness }
}

fn dummy_primitive(position: [f32; 3]) -> SdfPrimitive {
    SdfPrimitive {
        position,
        type_tag: TYPE_SPHERE,
        rotation: Quat::IDENTITY.to_array(),
        scale: [1.0; 3],
        _pad0: 0.0,
        params: [1.0, 0.0, 0.0, 0.0],
    }
}

#[test]
fn hash_is_stable_for_identical_inputs() {
    let items = vec![
        (0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE)),
        (1u32, Aabb::from_centre(Vec3::splat(5.0), Vec3::ONE)),
    ];
    let leaves = vec![
        dummy_leaf(ROLE_RAYMARCH_ADD, 0),
        dummy_leaf(ROLE_RAYMARCH_INT, 1),
    ];
    let payloads = vec![dummy_payload(0.0), dummy_payload(0.5)];
    let prims = vec![
        dummy_primitive([0.0, 0.0, 0.0]),
        dummy_primitive([5.0, 5.0, 5.0]),
    ];
    let h1 = BvhState::hash_scene(&items, &leaves, &payloads, &prims);
    let h2 = BvhState::hash_scene(&items, &leaves, &payloads, &prims);
    assert_eq!(h1, h2);
}

#[test]
fn hash_changes_when_aabb_changes() {
    let items_a = vec![(0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE))];
    let items_b = vec![(0u32, Aabb::from_centre(Vec3::X, Vec3::ONE))];
    let leaves = vec![dummy_leaf(ROLE_RAYMARCH_ADD, 0)];
    let payloads = vec![dummy_payload(0.0)];
    let prims = vec![dummy_primitive([0.0; 3])];
    assert_ne!(
        BvhState::hash_scene(&items_a, &leaves, &payloads, &prims),
        BvhState::hash_scene(&items_b, &leaves, &payloads, &prims),
    );
}

#[test]
fn hash_changes_when_flags_change() {
    let items = vec![(0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE))];
    let leaves_add = vec![dummy_leaf(ROLE_RAYMARCH_ADD, 0)];
    let leaves_int = vec![dummy_leaf(ROLE_RAYMARCH_INT, 0)];
    let payloads = vec![dummy_payload(0.5)];
    let prims = vec![dummy_primitive([0.0; 3])];
    assert_ne!(
        BvhState::hash_scene(&items, &leaves_add, &payloads, &prims),
        BvhState::hash_scene(&items, &leaves_int, &payloads, &prims),
    );
}

#[test]
fn hash_changes_when_entity_id_changes() {
    let items = vec![(0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE))];
    let leaves_a = vec![dummy_leaf(ROLE_RAYMARCH_ADD, 0)];
    let leaves_b = vec![dummy_leaf(ROLE_RAYMARCH_ADD, 7)];
    let payloads = vec![dummy_payload(0.0)];
    let prims = vec![dummy_primitive([0.0; 3])];
    assert_ne!(
        BvhState::hash_scene(&items, &leaves_a, &payloads, &prims),
        BvhState::hash_scene(&items, &leaves_b, &payloads, &prims),
    );
}

#[test]
fn hash_changes_when_smoothness_changes() {
    let items = vec![(0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE))];
    let leaves = vec![dummy_leaf(ROLE_RAYMARCH_ADD, 0)];
    let payloads_lo = vec![dummy_payload(0.1)];
    let payloads_hi = vec![dummy_payload(0.5)];
    let prims = vec![dummy_primitive([0.0; 3])];
    assert_ne!(
        BvhState::hash_scene(&items, &leaves, &payloads_lo, &prims),
        BvhState::hash_scene(&items, &leaves, &payloads_hi, &prims),
    );
}

#[test]
fn hash_changes_when_count_changes() {
    let leaves_one = vec![dummy_leaf(ROLE_RAYMARCH_ADD, 0)];
    let payloads_one = vec![dummy_payload(0.0)];
    let prims_one = vec![dummy_primitive([0.0; 3])];
    let items_one = vec![(0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE))];
    let items_two = vec![
        (0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE)),
        (1u32, Aabb::from_centre(Vec3::X, Vec3::ONE)),
    ];
    let leaves_two = vec![
        dummy_leaf(ROLE_RAYMARCH_ADD, 0),
        dummy_leaf(ROLE_RAYMARCH_ADD, 1),
    ];
    let payloads_two = vec![dummy_payload(0.0), dummy_payload(0.0)];
    let prims_two = vec![
        dummy_primitive([0.0; 3]),
        dummy_primitive([1.0, 0.0, 0.0]),
    ];
    assert_ne!(
        BvhState::hash_scene(&items_one, &leaves_one, &payloads_one, &prims_one),
        BvhState::hash_scene(&items_two, &leaves_two, &payloads_two, &prims_two),
    );
}

#[test]
fn hash_changes_when_primitive_rotation_changes() {
    // Pure-rotation move keeps `items[i].1` (the inflated AABB) and
    // every `leaf_aabbs[i]` field stable, but the rendered SDF still
    // differs because the fragment shader transforms the sample point
    // with `prim.rotation`. The hash MUST detect this — without
    // primitive bytes folded in, the kick gate would short-circuit
    // and the slot's `primitives_buffer` would render the pre-edit
    // pose forever.
    let items = vec![(0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE))];
    let leaves = vec![dummy_leaf(ROLE_RAYMARCH_ADD, 0)];
    let payloads = vec![dummy_payload(0.0)];

    let mut prim_a = dummy_primitive([0.0; 3]);
    prim_a.rotation = Quat::IDENTITY.to_array();
    let mut prim_b = dummy_primitive([0.0; 3]);
    prim_b.rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4).to_array();

    assert_ne!(
        BvhState::hash_scene(&items, &leaves, &payloads, &[prim_a]),
        BvhState::hash_scene(&items, &leaves, &payloads, &[prim_b]),
    );
}
