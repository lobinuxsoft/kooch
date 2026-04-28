//! CPU-only unit tests for [`super::BvhState::hash_scene`]. The hash
//! drives `kick_if_dirty`'s short-circuit on an unchanged scene, so any
//! field that participates in the rendered output must change the hash
//! when it changes — otherwise the BVH stays stale across edits.

use super::BvhState;
use crate::raymarch::instance::LeafAabb;
use glam::Vec3;
use ome_bvh::Aabb;

fn dummy_leaf(role: u32, smoothness: f32) -> LeafAabb {
    LeafAabb {
        aabb_min: [0.0; 3],
        role,
        aabb_max: [1.0; 3],
        smoothness,
    }
}

#[test]
fn hash_is_stable_for_identical_inputs() {
    let items = vec![
        (0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE)),
        (1u32, Aabb::from_centre(Vec3::splat(5.0), Vec3::ONE)),
    ];
    let leaves = vec![dummy_leaf(0, 0.0), dummy_leaf(1, 0.5)];
    let h1 = BvhState::hash_scene(&items, &leaves);
    let h2 = BvhState::hash_scene(&items, &leaves);
    assert_eq!(h1, h2);
}

#[test]
fn hash_changes_when_aabb_changes() {
    let items_a = vec![(0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE))];
    let items_b = vec![(0u32, Aabb::from_centre(Vec3::X, Vec3::ONE))];
    let leaves = vec![dummy_leaf(0, 0.0)];
    assert_ne!(
        BvhState::hash_scene(&items_a, &leaves),
        BvhState::hash_scene(&items_b, &leaves),
    );
}

#[test]
fn hash_changes_when_role_changes() {
    let items = vec![(0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE))];
    let leaves_add = vec![dummy_leaf(0, 0.5)];
    let leaves_int = vec![dummy_leaf(1, 0.5)];
    assert_ne!(
        BvhState::hash_scene(&items, &leaves_add),
        BvhState::hash_scene(&items, &leaves_int),
    );
}

#[test]
fn hash_changes_when_smoothness_changes() {
    let items = vec![(0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE))];
    let leaves_lo = vec![dummy_leaf(0, 0.1)];
    let leaves_hi = vec![dummy_leaf(0, 0.5)];
    assert_ne!(
        BvhState::hash_scene(&items, &leaves_lo),
        BvhState::hash_scene(&items, &leaves_hi),
    );
}

#[test]
fn hash_changes_when_count_changes() {
    let leaves = vec![dummy_leaf(0, 0.0)];
    let items_one = vec![(0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE))];
    let items_two = vec![
        (0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE)),
        (1u32, Aabb::from_centre(Vec3::X, Vec3::ONE)),
    ];
    let leaves_two = vec![dummy_leaf(0, 0.0), dummy_leaf(0, 0.0)];
    assert_ne!(
        BvhState::hash_scene(&items_one, &leaves),
        BvhState::hash_scene(&items_two, &leaves_two),
    );
}
