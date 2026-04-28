//! CPU-only unit tests for [`super::BvhState::hash_scene`]. The hash
//! drives `kick_if_dirty`'s short-circuit on an unchanged scene, so any
//! field that participates in the rendered output must change the hash
//! when it changes — otherwise the BVH stays stale across edits.

use super::BvhState;
use crate::raymarch::instance::RaymarchPayload;
use glam::Vec3;
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
    let h1 = BvhState::hash_scene(&items, &leaves, &payloads);
    let h2 = BvhState::hash_scene(&items, &leaves, &payloads);
    assert_eq!(h1, h2);
}

#[test]
fn hash_changes_when_aabb_changes() {
    let items_a = vec![(0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE))];
    let items_b = vec![(0u32, Aabb::from_centre(Vec3::X, Vec3::ONE))];
    let leaves = vec![dummy_leaf(ROLE_RAYMARCH_ADD, 0)];
    let payloads = vec![dummy_payload(0.0)];
    assert_ne!(
        BvhState::hash_scene(&items_a, &leaves, &payloads),
        BvhState::hash_scene(&items_b, &leaves, &payloads),
    );
}

#[test]
fn hash_changes_when_flags_change() {
    let items = vec![(0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE))];
    let leaves_add = vec![dummy_leaf(ROLE_RAYMARCH_ADD, 0)];
    let leaves_int = vec![dummy_leaf(ROLE_RAYMARCH_INT, 0)];
    let payloads = vec![dummy_payload(0.5)];
    assert_ne!(
        BvhState::hash_scene(&items, &leaves_add, &payloads),
        BvhState::hash_scene(&items, &leaves_int, &payloads),
    );
}

#[test]
fn hash_changes_when_entity_id_changes() {
    let items = vec![(0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE))];
    let leaves_a = vec![dummy_leaf(ROLE_RAYMARCH_ADD, 0)];
    let leaves_b = vec![dummy_leaf(ROLE_RAYMARCH_ADD, 7)];
    let payloads = vec![dummy_payload(0.0)];
    assert_ne!(
        BvhState::hash_scene(&items, &leaves_a, &payloads),
        BvhState::hash_scene(&items, &leaves_b, &payloads),
    );
}

#[test]
fn hash_changes_when_smoothness_changes() {
    let items = vec![(0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE))];
    let leaves = vec![dummy_leaf(ROLE_RAYMARCH_ADD, 0)];
    let payloads_lo = vec![dummy_payload(0.1)];
    let payloads_hi = vec![dummy_payload(0.5)];
    assert_ne!(
        BvhState::hash_scene(&items, &leaves, &payloads_lo),
        BvhState::hash_scene(&items, &leaves, &payloads_hi),
    );
}

#[test]
fn hash_changes_when_count_changes() {
    let leaves_one = vec![dummy_leaf(ROLE_RAYMARCH_ADD, 0)];
    let payloads_one = vec![dummy_payload(0.0)];
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
    assert_ne!(
        BvhState::hash_scene(&items_one, &leaves_one, &payloads_one),
        BvhState::hash_scene(&items_two, &leaves_two, &payloads_two),
    );
}
