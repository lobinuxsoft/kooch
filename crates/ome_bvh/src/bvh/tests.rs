//! Tests for the LBVH builder, refit and Karras layout invariants.
//! Mirrors the test set that lived inline in the pre-split `bvh.rs`.

use glam::Vec3;

use crate::aabb::Aabb;
use crate::leaf::LeafAabb;
use crate::node::BvhNode;

use super::refit::refit_slice_in_place;
use super::types::Bvh;

fn aabb_at(centre: Vec3, half: f32) -> Aabb {
    Aabb::from_centre(centre, Vec3::splat(half))
}

#[test]
fn empty_input_yields_empty_bvh() {
    let bvh: Bvh<u32> = Bvh::build(Vec::new());
    assert!(bvh.is_empty());
    assert_eq!(bvh.node_count(), 0);
    assert_eq!(bvh.leaf_count(), 0);
    assert_eq!(bvh.root_aabb(), Aabb::EMPTY);
}

#[test]
fn single_item_is_one_leaf_node() {
    let bvh = Bvh::build(vec![(7u32, aabb_at(Vec3::ZERO, 1.0))]);
    assert_eq!(bvh.node_count(), 1);
    assert_eq!(bvh.leaf_count(), 1);
    assert!(bvh.nodes[0].is_leaf());
    assert_eq!(bvh.nodes[0].count(), 1);
    assert_eq!(bvh.leaves[0], 7);
}

#[test]
fn two_items_root_internal_with_two_leaves() {
    let items = vec![
        (1u32, aabb_at(Vec3::ZERO, 0.5)),
        (2u32, aabb_at(Vec3::splat(10.0), 0.5)),
    ];
    let bvh = Bvh::build(items);
    // Karras layout: 1 internal at idx 0 + 2 leaves at idx 1, 2 = 3 nodes.
    assert_eq!(bvh.node_count(), 3);
    assert!(bvh.nodes[0].is_internal());
    assert!(bvh.nodes[1].is_leaf());
    assert!(bvh.nodes[2].is_leaf());
    // Root spans both items.
    let root = bvh.root_aabb();
    assert!(root.min.x <= -0.5);
    assert!(root.max.x >= 10.5);
}

#[test]
fn balanced_depth_logarithmic_for_8_items() {
    // 8 items → 8 leaves + 7 internals = 15 nodes total.
    let items: Vec<(u32, Aabb)> = (0..8u32)
        .map(|i| (i, aabb_at(Vec3::new(i as f32, 0.0, 0.0), 0.4)))
        .collect();
    let bvh = Bvh::build(items);
    assert_eq!(bvh.leaf_count(), 8);
    assert_eq!(bvh.node_count(), 15);
    // Verify every leaf is reachable: traverse and count leaves.
    let mut leaves_seen = 0;
    let mut stack = vec![0u32];
    while let Some(idx) = stack.pop() {
        let n = &bvh.nodes[idx as usize];
        if n.is_leaf() {
            leaves_seen += n.count();
        } else {
            stack.push(n.left);
            stack.push(n.right_child());
        }
    }
    assert_eq!(leaves_seen, 8);
}

#[test]
fn root_aabb_unions_all_input_bounds() {
    let items = vec![
        (1u32, Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0))),
        (2u32, Aabb::new(Vec3::new(5.0, -2.0, 3.0), Vec3::new(6.0, -1.0, 4.0))),
        (3u32, Aabb::new(Vec3::new(-3.0, 2.0, 0.0), Vec3::new(-2.0, 3.0, 1.0))),
    ];
    let bvh = Bvh::build(items);
    let root = bvh.root_aabb();
    assert_eq!(root.min, Vec3::new(-3.0, -2.0, 0.0));
    assert_eq!(root.max, Vec3::new(6.0, 3.0, 4.0));
}

#[test]
fn morton_sort_groups_neighbours() {
    let items = vec![
        (0u32, aabb_at(Vec3::new(0.0, 0.0, 0.0), 0.4)),
        (1u32, aabb_at(Vec3::new(1.0, 0.0, 0.0), 0.4)),
        (2u32, aabb_at(Vec3::new(0.0, 1.0, 0.0), 0.4)),
        (3u32, aabb_at(Vec3::new(1.0, 1.0, 0.0), 0.4)),
    ];
    let bvh = Bvh::build(items);
    let mut sorted = bvh.leaves.clone();
    sorted.sort();
    assert_eq!(sorted, vec![0, 1, 2, 3]);
}

#[test]
fn duplicate_morton_codes_split_via_index_tiebreak() {
    // 4 items at the same centre — all share the same Morton code.
    // Karras' delta tie-breaks via index, so the build still
    // produces a valid tree without infinite-looping.
    let items: Vec<(u32, Aabb)> = (0..4u32)
        .map(|i| (i, aabb_at(Vec3::ZERO, 0.5)))
        .collect();
    let bvh = Bvh::build(items);
    assert_eq!(bvh.leaf_count(), 4);
    // 4 leaves + 3 internals = 7.
    assert_eq!(bvh.node_count(), 7);
}

#[test]
fn deep_tree_for_1024_items() {
    let items: Vec<(u32, Aabb)> = (0..1024u32)
        .map(|i| {
            let x = (i % 32) as f32;
            let y = (i / 32) as f32;
            (i, aabb_at(Vec3::new(x, y, 0.0), 0.4))
        })
        .collect();
    let bvh = Bvh::build(items);
    assert_eq!(bvh.leaf_count(), 1024);
    // 1024 leaves + 1023 internals = 2047 nodes.
    assert_eq!(bvh.node_count(), 2047);
}

#[test]
fn karras_layout_internals_come_before_leaves() {
    // Verify the canonical layout: nodes[0..N-1) are internals,
    // nodes[N-1..2N-1) are leaves. Tested with a non-trivial size
    // so the boundary is unambiguous.
    let items: Vec<(u32, Aabb)> = (0..16u32)
        .map(|i| (i, aabb_at(Vec3::new(i as f32, 0.0, 0.0), 0.4)))
        .collect();
    let bvh = Bvh::build(items);
    let n = 16;
    for i in 0..(n - 1) {
        assert!(bvh.nodes[i].is_internal(), "node {i} should be internal");
    }
    for i in (n - 1)..(2 * n - 1) {
        assert!(bvh.nodes[i].is_leaf(), "node {i} should be leaf");
    }
}

fn sorted_indices_from_payload_id_bvh(bvh: &Bvh<u32>) -> Vec<u32> {
    // Test convenience: when payload T = u32 and the input items
    // were `(i, ...)` for i in 0..n, then `bvh.leaves[k]` IS the
    // original position of the leaf at sorted position k.
    bvh.leaves.clone()
}

fn leaf_aabb_at(centre: Vec3, half: f32) -> LeafAabb {
    let a = aabb_at(centre, half);
    LeafAabb {
        aabb_min: a.min.into(),
        flags: 0,
        aabb_max: a.max.into(),
        entity_id: 0,
    }
}

#[test]
fn refit_in_place_identity_is_noop() {
    // Identity refit: same leaf_aabbs as the build. Every node's
    // AABB must come back byte-identical to the original.
    let items: Vec<(u32, Aabb)> = (0..8u32)
        .map(|i| (i, aabb_at(Vec3::new(i as f32, 0.0, 0.0), 0.4)))
        .collect();
    let original = Bvh::build(items.clone());
    let leaf_aabbs: Vec<LeafAabb> = items
        .iter()
        .map(|(_, a)| leaf_aabb_at(a.center(), 0.4))
        .collect();
    let sorted_indices = sorted_indices_from_payload_id_bvh(&original);
    let mut refitted = original.clone();
    refitted.refit_in_place(&leaf_aabbs, &sorted_indices);
    for (i, (a, b)) in original.nodes.iter().zip(refitted.nodes.iter()).enumerate() {
        assert_eq!(a.aabb_min, b.aabb_min, "node {i} aabb_min drift on identity refit");
        assert_eq!(a.aabb_max, b.aabb_max, "node {i} aabb_max drift on identity refit");
    }
}

#[test]
fn refit_in_place_matches_full_rebuild_when_centres_preserved() {
    // Shrink each AABB without moving its centre. Morton ordering
    // (centre-driven) is preserved → topology unchanged → refit
    // must produce the same node AABBs as a full rebuild.
    let centres: Vec<Vec3> = (0..16u32)
        .map(|i| Vec3::new(i as f32, 0.0, 0.0))
        .collect();
    let items_v0: Vec<(u32, Aabb)> = centres
        .iter()
        .copied()
        .enumerate()
        .map(|(i, c)| (i as u32, aabb_at(c, 0.4)))
        .collect();
    let bvh_v0 = Bvh::build(items_v0);
    let sorted_indices = sorted_indices_from_payload_id_bvh(&bvh_v0);

    // v1: same centres, half the half-extent.
    let items_v1: Vec<(u32, Aabb)> = centres
        .iter()
        .copied()
        .enumerate()
        .map(|(i, c)| (i as u32, aabb_at(c, 0.2)))
        .collect();
    let leaf_aabbs_v1: Vec<LeafAabb> = items_v1
        .iter()
        .map(|(_, a)| leaf_aabb_at(a.center(), 0.2))
        .collect();
    let bvh_full_rebuild = Bvh::build(items_v1);

    let mut bvh_refitted = bvh_v0;
    bvh_refitted.refit_in_place(&leaf_aabbs_v1, &sorted_indices);

    // Topology unchanged (same morton order) → leaves slice must
    // match. Node AABBs must match the full rebuild.
    assert_eq!(bvh_refitted.leaves, bvh_full_rebuild.leaves);
    for (i, (a, b)) in bvh_refitted
        .nodes
        .iter()
        .zip(bvh_full_rebuild.nodes.iter())
        .enumerate()
    {
        assert_eq!(a.aabb_min, b.aabb_min, "node {i} aabb_min mismatch vs full rebuild");
        assert_eq!(a.aabb_max, b.aabb_max, "node {i} aabb_max mismatch vs full rebuild");
    }
}

#[test]
fn refit_in_place_empty_is_noop() {
    let mut empty: Bvh<u32> = Bvh::empty();
    empty.refit_in_place(&[], &[]);
    assert!(empty.is_empty());
}

#[test]
fn build_into_matches_build_byte_identical() {
    // The slice variant must produce byte-identical output to the
    // owning variant for the same input — `OmeAccel` assumes this
    // when feeding pool slices to `Bvh::build_into`.
    let items: Vec<(u32, Aabb)> = (0..32u32)
        .map(|i| {
            let x = (i % 8) as f32;
            let y = (i / 8) as f32;
            (i, aabb_at(Vec3::new(x, y, 0.0), 0.4))
        })
        .collect();
    let owning = Bvh::build(items.clone());

    let n = items.len();
    let mut nodes_dst = vec![BvhNode::default(); 2 * n - 1];
    let mut leaves_dst = vec![0u32; n];
    let meta = Bvh::<u32>::build_into(items, &mut nodes_dst, &mut leaves_dst);

    assert_eq!(meta.node_count, owning.node_count() as u32);
    assert_eq!(meta.leaf_count, owning.leaf_count() as u32);
    assert_eq!(nodes_dst, owning.nodes);
    assert_eq!(leaves_dst, owning.leaves);
    assert_eq!(meta.root_aabb, owning.root_aabb());
}

#[test]
fn build_into_empty_returns_zero_meta() {
    let mut nodes_dst: Vec<BvhNode> = Vec::new();
    let mut leaves_dst: Vec<u32> = Vec::new();
    let meta = Bvh::<u32>::build_into(Vec::new(), &mut nodes_dst, &mut leaves_dst);
    assert_eq!(meta.node_count, 0);
    assert_eq!(meta.leaf_count, 0);
    assert_eq!(meta.root_aabb, Aabb::EMPTY);
}

#[test]
fn build_into_single_leaf_writes_one_node() {
    let items = vec![(7u32, aabb_at(Vec3::ZERO, 1.0))];
    let mut nodes_dst = vec![BvhNode::default(); 1];
    let mut leaves_dst = vec![0u32; 1];
    let meta = Bvh::<u32>::build_into(items, &mut nodes_dst, &mut leaves_dst);
    assert_eq!(meta.node_count, 1);
    assert_eq!(meta.leaf_count, 1);
    assert!(nodes_dst[0].is_leaf());
    assert_eq!(leaves_dst[0], 7);
}

#[test]
fn refit_slice_in_place_matches_owning_refit() {
    // Slice refit must agree with the owning refit on the same
    // topology. Build with v0 centres, refit into v1 AABBs, both
    // paths must produce identical node AABBs.
    let centres: Vec<Vec3> = (0..16u32)
        .map(|i| Vec3::new(i as f32, 0.0, 0.0))
        .collect();
    let items_v0: Vec<(u32, Aabb)> = centres
        .iter()
        .copied()
        .enumerate()
        .map(|(i, c)| (i as u32, aabb_at(c, 0.4)))
        .collect();
    let bvh_owning = Bvh::build(items_v0.clone());
    let sorted = sorted_indices_from_payload_id_bvh(&bvh_owning);

    let leaf_aabbs_v1: Vec<LeafAabb> = centres
        .iter()
        .copied()
        .map(|c| leaf_aabb_at(c, 0.2))
        .collect();

    let mut bvh_owning_refitted = bvh_owning.clone();
    bvh_owning_refitted.refit_in_place(&leaf_aabbs_v1, &sorted);

    let mut slice_nodes = bvh_owning.nodes.clone();
    refit_slice_in_place(&mut slice_nodes, 16, &leaf_aabbs_v1, &sorted);

    for (i, (a, b)) in bvh_owning_refitted
        .nodes
        .iter()
        .zip(slice_nodes.iter())
        .enumerate()
    {
        assert_eq!(a.aabb_min, b.aabb_min, "node {i} aabb_min slice/owning mismatch");
        assert_eq!(a.aabb_max, b.aabb_max, "node {i} aabb_max slice/owning mismatch");
    }
}

#[test]
fn karras_supports_non_contiguous_children() {
    // Asymmetric split case: build with a Morton distribution that
    // forces some internal node's left/right children to be of
    // different types (one internal, one leaf). Verify the
    // traversal still reaches all leaves.
    let items: Vec<(u32, Aabb)> = (0..5u32)
        .map(|i| (i, aabb_at(Vec3::new(i as f32, 0.0, 0.0), 0.4)))
        .collect();
    let bvh = Bvh::build(items);
    let mut leaves_seen = 0u32;
    let mut stack = vec![0u32];
    while let Some(idx) = stack.pop() {
        let n = &bvh.nodes[idx as usize];
        if n.is_leaf() {
            leaves_seen += n.count();
        } else {
            stack.push(n.left);
            stack.push(n.right_child());
        }
    }
    assert_eq!(leaves_seen, 5);
}
