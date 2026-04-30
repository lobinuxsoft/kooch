//! WGSL-contract regression tests for the BLAS streaming pipeline.
//!
//! Pure CPU — no `wgpu::Device` required — so they always run, including
//! in the CI fallback that skips the GPU tests. Pin the two invariants
//! the WGSL traversal of `OmeAccel` depends on:
//!
//! 1. After `Bvh::build_into` + the streaming post-pass, every BLAS
//!    leaf's `node.left` carries the **absolute** pool primitive index
//!    (the value `leaves_scratch[k]`), not the sorted-position `k`.
//! 2. `Bvh::build_into` itself emits `node.left = k` in its current
//!    convention — pinned so a future builder change surfaces here
//!    instead of silently breaking the WGSL path through OmeAccel.

use crate::aabb::Aabb;
use crate::bvh::Bvh;
use crate::leaf::LeafAabb;
use crate::node::BvhNode;
use glam::Vec3;

fn aabb_leaf(centre_x: f32, entity_id: u32) -> LeafAabb {
    LeafAabb {
        aabb_min: [centre_x - 0.5, -0.5, -0.5],
        flags: 0,
        aabb_max: [centre_x + 0.5, 0.5, 0.5],
        entity_id,
    }
}

#[test]
fn leaf_left_field_is_absolute_pool_index_after_post_pass() {
    // Mimic `insert_chunk`'s build step on a 4-primitive chunk
    // starting at `first_primitive = 100`.
    let first_primitive = 100u32;
    let leaf_aabbs = [
        aabb_leaf(0.0, 0),
        aabb_leaf(10.0, 1),
        aabb_leaf(20.0, 2),
        aabb_leaf(30.0, 3),
    ];
    let n = leaf_aabbs.len() as u32;
    let total_nodes = 2 * n - 1;
    let items: Vec<(u32, Aabb)> = leaf_aabbs
        .iter()
        .enumerate()
        .map(|(i, l)| {
            (
                first_primitive + i as u32,
                Aabb::new(Vec3::from(l.aabb_min), Vec3::from(l.aabb_max)),
            )
        })
        .collect();
    let mut nodes = vec![BvhNode::default(); total_nodes as usize];
    let mut leaves = vec![0u32; n as usize];
    Bvh::<u32>::build_into(items, &mut nodes, &mut leaves);

    // Pre-condition: `Bvh::build_into` writes leaf-node `left = k` by
    // default. Pinned so a future builder change surfaces here.
    let leaf_offset = (n - 1) as usize;
    for k in 0..n as usize {
        assert_eq!(
            nodes[leaf_offset + k].left,
            k as u32,
            "Bvh::build_into convention changed: pre-pass leaf.left was \
             expected to be sorted-position k",
        );
    }

    // Post-pass — same logic as `insert_chunk` / `refit_chunk`.
    for k in 0..n as usize {
        nodes[leaf_offset + k].left = leaves[k];
    }

    // Post-condition: every leaf's `left` is an absolute pool primitive
    // index in `[first_primitive, first_primitive + n)`. The set of
    // `left` values equals the set of pool indices.
    let mut got: Vec<u32> = (0..n as usize)
        .map(|k| nodes[leaf_offset + k].left)
        .collect();
    got.sort();
    let expected: Vec<u32> = (first_primitive..first_primitive + n).collect();
    assert_eq!(
        got, expected,
        "post-pass must permute leaf.left into the absolute-index set"
    );

    // Bonus invariant: leaves[k] carries the absolute index, and
    // post-pass copies it into nodes[leaf_offset + k].left.
    for k in 0..n as usize {
        assert_eq!(nodes[leaf_offset + k].left, leaves[k]);
    }
}

/// Single-primitive chunks are the degenerate case `n == 1` →
/// `total_nodes == 1`, `leaf_offset == 0`, the only node is the root
/// *and* the leaf. Post-pass must still rewrite its `left`.
#[test]
fn leaf_left_post_pass_handles_single_primitive_chunk() {
    let first_primitive = 42u32;
    let leaf = aabb_leaf(0.0, 7);
    let items = vec![(
        first_primitive,
        Aabb::new(Vec3::from(leaf.aabb_min), Vec3::from(leaf.aabb_max)),
    )];
    let mut nodes = vec![BvhNode::default(); 1];
    let mut leaves = vec![0u32; 1];
    Bvh::<u32>::build_into(items, &mut nodes, &mut leaves);

    // Pre: build_into writes node[0].left = 0 for the lone leaf.
    assert_eq!(nodes[0].left, 0);
    // Post-pass.
    nodes[0].left = leaves[0];
    assert_eq!(nodes[0].left, first_primitive);
    assert!(nodes[0].is_leaf());
}
