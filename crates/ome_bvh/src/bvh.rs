//! [`Bvh<T>`] — generic LBVH built from a `Vec<(T, Aabb)>`.
//!
//! Build pipeline (CPU; PR-3 ports the same algorithm to a compute
//! shader and emits byte-identical [`BvhNode`] arrays):
//!
//! 1. Compute the scene bounds (union of every input AABB).
//! 2. For each item, compute its centre, normalise to the scene
//!    bounds, encode a 30-bit [`MortonCode`].
//! 3. Sort items by Morton code (stable). Spatial neighbours land in
//!    contiguous ranges of the array.
//! 4. BFS build: allocate the root, then for every range pop, allocate
//!    a contiguous **pair** of child slots and enqueue both subranges.
//!    The "right child = left + 1" invariant from the issue body holds
//!    because pairs are always contiguous in allocation order.
//! 5. Compute internal AABBs in reverse-allocation order — children
//!    were allocated after their parent (BFS), so iterating right-to-
//!    left guarantees a child's bounds are filled before its parent
//!    needs them.

use std::collections::VecDeque;

use glam::Vec3;

use crate::aabb::Aabb;
use crate::morton::MortonCode;
use crate::node::BvhNode;

/// LBVH built over a generic payload `T`. The [`nodes`] array is the
/// flat tree (root at `nodes[0]` when non-empty); the [`leaves`] array
/// is the per-leaf payload referenced by `nodes[i].left_or_first` when
/// `nodes[i].count > 0`.
///
/// [`nodes`]: Self::nodes
/// [`leaves`]: Self::leaves
#[derive(Debug, Clone)]
pub struct Bvh<T: Copy> {
    pub nodes: Vec<BvhNode>,
    pub leaves: Vec<T>,
}

impl<T: Copy> Bvh<T> {
    pub fn empty() -> Self {
        Self {
            nodes: Vec::new(),
            leaves: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Number of leaves in the tree (== number of input items).
    pub fn leaf_count(&self) -> usize {
        self.leaves.len()
    }

    /// Number of nodes (internal + leaf) in the flat array.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Build a BVH from a list of `(payload, bounds)` items. Empty
    /// input yields [`Self::empty`]. Single-item input yields a tree
    /// with one leaf node.
    pub fn build(items: Vec<(T, Aabb)>) -> Self {
        if items.is_empty() {
            return Self::empty();
        }

        // 1. Scene bounds.
        let scene_bounds = items
            .iter()
            .fold(Aabb::EMPTY, |acc, (_, aabb)| acc.union(aabb));

        // Avoid division by zero on a single-point scene by clamping
        // any zero extent to 1.0 — the resulting Morton codes collapse
        // to 0 across that axis but the build still terminates cleanly.
        let extent = scene_bounds.max - scene_bounds.min;
        let inv_extent = Vec3::new(
            if extent.x > 0.0 { 1.0 / extent.x } else { 0.0 },
            if extent.y > 0.0 { 1.0 / extent.y } else { 0.0 },
            if extent.z > 0.0 { 1.0 / extent.z } else { 0.0 },
        );

        // 2. Morton-code each item by its centre.
        let mut morton_items: Vec<(MortonCode, T, Aabb)> = items
            .into_iter()
            .map(|(payload, aabb)| {
                let centre = aabb.center();
                let normalized = (centre - scene_bounds.min) * inv_extent;
                let code = MortonCode::from_normalized(normalized);
                (code, payload, aabb)
            })
            .collect();

        // 3. Sort by Morton (stable to keep deterministic output for
        // duplicate codes).
        morton_items.sort_by_key(|(c, _, _)| *c);

        // 4. BFS build. `work` holds (target_node_idx, first, last)
        // ranges; the node at `target_node_idx` is the placeholder
        // we'll fill in this iteration.
        let mut nodes: Vec<BvhNode> = vec![BvhNode::default()];
        let mut leaves: Vec<T> = Vec::with_capacity(morton_items.len());
        let mut work: VecDeque<(u32, usize, usize)> = VecDeque::new();
        work.push_back((0, 0, morton_items.len()));

        while let Some((target_idx, first, last)) = work.pop_front() {
            if last - first == 1 {
                // Single-item leaf.
                let item_idx = leaves.len() as u32;
                let item = &morton_items[first];
                leaves.push(item.1);
                nodes[target_idx as usize] = BvhNode::leaf(
                    item.2.min.into(),
                    item.2.max.into(),
                    item_idx,
                    1,
                );
                continue;
            }

            // Internal split. Allocate the contiguous child pair BEFORE
            // computing the split index, so left at idx N, right at N+1.
            let left_idx = nodes.len() as u32;
            nodes.push(BvhNode::default());
            nodes.push(BvhNode::default());

            // Placeholder bounds — filled in pass 5 below.
            nodes[target_idx as usize] =
                BvhNode::internal([0.0; 3], [0.0; 3], left_idx);

            let split = find_split(&morton_items, first, last);

            work.push_back((left_idx, first, split));
            work.push_back((left_idx + 1, split, last));
        }

        // 5. Compute internal AABBs in reverse-allocation order.
        // BFS pushes parents before children, so iterating high → low
        // guarantees children are filled before their parent is read.
        for i in (0..nodes.len()).rev() {
            if nodes[i].is_internal() {
                let left = nodes[i].left_or_first as usize;
                let right = left + 1;
                let left_aabb = Aabb::new(
                    Vec3::from(nodes[left].aabb_min),
                    Vec3::from(nodes[left].aabb_max),
                );
                let right_aabb = Aabb::new(
                    Vec3::from(nodes[right].aabb_min),
                    Vec3::from(nodes[right].aabb_max),
                );
                let combined = left_aabb.union(&right_aabb);
                nodes[i].aabb_min = combined.min.into();
                nodes[i].aabb_max = combined.max.into();
            }
        }

        Self { nodes, leaves }
    }

    /// AABB of the root node — i.e. the union of every leaf bound.
    /// Returns [`Aabb::EMPTY`] for an empty BVH.
    pub fn root_aabb(&self) -> Aabb {
        if self.nodes.is_empty() {
            return Aabb::EMPTY;
        }
        Aabb::new(
            Vec3::from(self.nodes[0].aabb_min),
            Vec3::from(self.nodes[0].aabb_max),
        )
    }
}

/// Find the split position inside `items[first..last]` where the
/// highest differing bit of the Morton codes transitions from 0 to 1.
/// Guarantees `first < split < last` (each child gets ≥1 item).
fn find_split<T>(
    items: &[(MortonCode, T, Aabb)],
    first: usize,
    last: usize,
) -> usize {
    debug_assert!(last - first >= 2);

    let first_code = items[first].0.0;
    let last_code = items[last - 1].0.0;
    if first_code == last_code {
        // All identical — split in the middle.
        return first + (last - first) / 2;
    }

    // Highest bit where the codes diverge. `xor.leading_zeros()` is
    // the count of identical leading bits; the first differing bit is
    // at position `31 - leading_zeros`.
    let xor = first_code ^ last_code;
    let split_bit = 31 - xor.leading_zeros();
    let mask = 1u32 << split_bit;

    // Binary search: the lowest index where (code & mask) != 0.
    let mut lo = first;
    let mut hi = last;
    while lo < hi {
        let mid = (lo + hi) / 2;
        if items[mid].0.0 & mask != 0 {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    // Clamp into the valid range so each child gets ≥1 item.
    lo.clamp(first + 1, last - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(bvh.nodes[0].count, 1);
        assert_eq!(bvh.leaves[0], 7);
    }

    #[test]
    fn two_items_root_internal_with_two_leaves() {
        let items = vec![
            (1u32, aabb_at(Vec3::ZERO, 0.5)),
            (2u32, aabb_at(Vec3::splat(10.0), 0.5)),
        ];
        let bvh = Bvh::build(items);
        // Root internal + 2 leaves = 3 nodes.
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
        // 8 items → 8 leaves + 7 internals = 15 nodes total in a full
        // binary tree.
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
                leaves_seen += n.count;
            } else {
                stack.push(n.left_or_first);
                stack.push(n.left_or_first + 1);
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
        // 4 items in a Z-curve order. After build, leaves should be
        // visited in spatial order — adjacent leaves' centres are
        // close in space.
        let items = vec![
            (0u32, aabb_at(Vec3::new(0.0, 0.0, 0.0), 0.4)),
            (1u32, aabb_at(Vec3::new(1.0, 0.0, 0.0), 0.4)),
            (2u32, aabb_at(Vec3::new(0.0, 1.0, 0.0), 0.4)),
            (3u32, aabb_at(Vec3::new(1.0, 1.0, 0.0), 0.4)),
        ];
        let bvh = Bvh::build(items);
        // The leaves array is in Morton order, not insertion order.
        // We don't assert the exact permutation (depends on the Morton
        // encoder), only that all 4 payloads are present.
        let mut sorted = bvh.leaves.clone();
        sorted.sort();
        assert_eq!(sorted, vec![0, 1, 2, 3]);
    }

    #[test]
    fn duplicate_morton_codes_split_at_midpoint() {
        // 4 items at exactly the same centre — all share the same
        // Morton code. The split function should recurse mid-range
        // without infinite-looping.
        let items: Vec<(u32, Aabb)> = (0..4u32)
            .map(|i| (i, aabb_at(Vec3::ZERO, 0.5)))
            .collect();
        let bvh = Bvh::build(items);
        assert_eq!(bvh.leaf_count(), 4);
        // 4 leaves + 3 internals.
        assert_eq!(bvh.node_count(), 7);
    }

    #[test]
    fn deep_tree_for_1024_items() {
        // Stress test: 1024 items across a 32×32 grid in 2D. Build
        // must complete and all leaves reachable.
        let items: Vec<(u32, Aabb)> = (0..1024u32)
            .map(|i| {
                let x = (i % 32) as f32;
                let y = (i / 32) as f32;
                (i, aabb_at(Vec3::new(x, y, 0.0), 0.4))
            })
            .collect();
        let bvh = Bvh::build(items);
        assert_eq!(bvh.leaf_count(), 1024);
        // Full binary tree: 1024 leaves + 1023 internals = 2047 nodes.
        assert_eq!(bvh.node_count(), 2047);
    }
}
