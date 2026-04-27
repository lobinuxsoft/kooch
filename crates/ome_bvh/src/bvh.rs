//! [`Bvh<T>`] — generic LBVH built from a `Vec<(T, Aabb)>` via the
//! Karras 2012 parallel construction algorithm.
//!
//! The CPU build mirrors the WGSL compute build (PR-3 of #115)
//! byte-for-byte: same node layout, same indexing convention, same
//! AABB propagation order. `Bvh::build` and `Bvh::build_gpu` produce
//! identical [`BvhNode`] arrays for the same input.
//!
//! # Layout (Karras-canonical, 2N-1 nodes)
//!
//! - `nodes[0..N-1)`     → internal nodes (N-1 of them).
//! - `nodes[N-1..2N-1)`  → leaves (N of them, in Morton order).
//!
//! Internal node `i`'s left and right children may be at arbitrary
//! positions in the flat array (one may be internal, the other a
//! leaf). The "right child = left + 1" invariant from the BFS-era
//! layout is gone — both indices are stored explicitly via
//! [`BvhNode::left`] and [`BvhNode::right_child`].
//!
//! # Build pipeline
//!
//! 1. Compute the scene bounds (union of every input AABB).
//! 2. For each item, compute its centre, normalise to the scene
//!    bounds, encode a 30-bit [`MortonCode`].
//! 3. Sort items by Morton code (stable). Spatial neighbours land in
//!    contiguous ranges of the array.
//! 4. Write all N leaves into `nodes[N-1..2N-1)` in Morton order.
//! 5. For each internal `i ∈ [0, N-1)`, run Karras' algorithm to
//!    determine its range `[first, last]` and split position `γ`,
//!    then write `nodes[i] = internal(left_child, right_child)`.
//! 6. Bottom-up AABB propagation from the root via post-order DFS.

use glam::Vec3;

use crate::aabb::Aabb;
use crate::morton::MortonCode;
use crate::node::BvhNode;

/// LBVH built over a generic payload `T`. The [`nodes`] array is the
/// flat tree (root at `nodes[0]` when non-empty); the [`leaves`] array
/// is the per-leaf payload referenced by `nodes[i].first_leaf()` when
/// `nodes[i].is_leaf()`.
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
    /// with one leaf node at `nodes[0]`.
    pub fn build(items: Vec<(T, Aabb)>) -> Self {
        if items.is_empty() {
            return Self::empty();
        }
        let n = items.len();

        // 1. Scene bounds.
        let scene_bounds = items
            .iter()
            .fold(Aabb::EMPTY, |acc, (_, aabb)| acc.union(aabb));

        // Avoid division by zero on a single-point scene by clamping
        // any zero extent — the resulting Morton codes collapse to 0
        // across that axis but the build still terminates cleanly.
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

        // Karras-canonical layout: 2N-1 nodes total (or 1 when N==1).
        // Internals at [0, N-1), leaves at [N-1, 2N-1).
        let leaf_offset = n.saturating_sub(1);
        let total_nodes = 2 * n - 1;
        let mut nodes: Vec<BvhNode> = vec![BvhNode::default(); total_nodes];
        let leaves: Vec<T> = morton_items.iter().map(|(_, p, _)| *p).collect();

        // 4. Write leaves into [leaf_offset, total_nodes).
        for (k, item) in morton_items.iter().enumerate() {
            let aabb = &item.2;
            nodes[leaf_offset + k] = BvhNode::leaf(
                aabb.min.into(),
                aabb.max.into(),
                k as u32,
                1,
            );
        }

        // 5. Compute internals via Karras (skipped when N==1: no
        // internal nodes, the single leaf at nodes[0] is the root).
        if n > 1 {
            let morton: Vec<u32> = morton_items.iter().map(|(c, _, _)| c.0).collect();
            for i in 0..(n - 1) {
                let (first, last, gamma) = karras_range_and_split(&morton, i);
                let left_idx = if gamma == first {
                    // Single-leaf left child.
                    (leaf_offset + gamma) as u32
                } else {
                    // Internal at position γ.
                    gamma as u32
                };
                let right_idx = if gamma + 1 == last {
                    (leaf_offset + gamma + 1) as u32
                } else {
                    (gamma + 1) as u32
                };
                nodes[i] = BvhNode::internal([0.0; 3], [0.0; 3], left_idx, right_idx);
            }

            // 6. Bottom-up AABB propagation from root via post-order DFS.
            propagate_aabb(&mut nodes, 0);
        }

        Self { nodes, leaves }
    }

    /// Build a BVH on the GPU. Chains morton encoding + onesweep radix
    /// sort + Karras parallel construction on a single command encoder
    /// and submits in one call. Returns a [`crate::gpu::BvhGpuBuild`]
    /// handle the caller polls (frame-loop-friendly) until the result
    /// is ready. See the module docs in `gpu/build.rs` for usage
    /// patterns (CPU readback vs GPU-resident handoff).
    pub fn build_gpu(
        builder: &mut crate::gpu::BvhGpuBuilder,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        items: Vec<(T, Aabb)>,
    ) -> crate::gpu::BvhGpuBuild<T>
    where
        T: bytemuck::Pod,
    {
        crate::gpu::build::build_gpu(builder, device, queue, items)
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

/// Karras' delta function: longest common prefix length between
/// `morton[i]` and `morton[j]`, treating equal codes as resolved by
/// appending the 32-bit index (the index tie-break makes the
/// algorithm well-defined when multiple items share a Morton code).
///
/// Returns `-1` when `j` is out of range, signalling "no neighbour
/// in this direction" to the caller.
fn delta(morton: &[u32], i: usize, j: i64) -> i32 {
    let n = morton.len() as i64;
    if j < 0 || j >= n {
        return -1;
    }
    let xi = morton[i];
    let xj = morton[j as usize];
    if xi == xj {
        // All 32 morton bits equal — tie-break with index.
        return 32 + (i as u32 ^ j as u32).leading_zeros() as i32;
    }
    (xi ^ xj).leading_zeros() as i32
}

/// Run Karras' construction algorithm for one internal node `i`.
/// Returns `(first, last, gamma)` where `[first, last]` is the range
/// of leaves covered by the subtree and `gamma` is the split position
/// (left covers `[first, gamma]`, right covers `[gamma+1, last]`).
fn karras_range_and_split(morton: &[u32], i: usize) -> (usize, usize, usize) {
    let i_s = i as i64;

    // Direction d ∈ {-1, +1}: which side of i extends the range.
    let delta_plus = delta(morton, i, i_s + 1);
    let delta_minus = delta(morton, i, i_s - 1);
    let d: i64 = if delta_plus > delta_minus { 1 } else { -1 };

    // Lower bound on the common prefix shared by every leaf in this
    // node's range — the "other end" must have a strictly longer
    // common prefix than the leaf one step in the opposite direction.
    let delta_min = delta(morton, i, i_s - d);

    // Exponential search to find an upper bound l_max on the range
    // length. Doubles until the leaf at i + l_max*d falls below
    // delta_min (or out of range).
    let mut l_max: i64 = 2;
    while delta(morton, i, i_s + l_max * d) > delta_min {
        l_max *= 2;
    }

    // Binary search inside [0, l_max) for the exact length l such
    // that delta(i, i + l*d) > delta_min and delta(i, i + (l+1)*d) ≤ delta_min.
    let mut l: i64 = 0;
    let mut t = l_max / 2;
    while t > 0 {
        if delta(morton, i, i_s + (l + t) * d) > delta_min {
            l += t;
        }
        t /= 2;
    }
    let j = i_s + l * d;

    // Split position γ: largest s ∈ [0, l) such that
    // delta(i, i + s*d) > delta_node, where delta_node = delta(i, j).
    let delta_node = delta(morton, i, j);
    let mut s: i64 = 0;
    let mut div: i64 = 2;
    loop {
        let t = ((l as f64) / (div as f64)).ceil() as i64;
        if delta(morton, i, i_s + (s + t) * d) > delta_node {
            s += t;
        }
        if t <= 1 {
            break;
        }
        div *= 2;
    }

    // For d=-1 the split lies one step further "left" so that the
    // returned γ is the inclusive end of the left child's range.
    let gamma = (i_s + s * d + d.min(0)) as usize;
    let first = i_s.min(j) as usize;
    let last = i_s.max(j) as usize;
    (first, last, gamma)
}

/// Recursive post-order DFS that propagates child AABBs into their
/// parent. Returns the AABB of the subtree rooted at `idx` so the
/// parent can read it without re-fetching from the array.
///
/// Stack depth is bounded by tree depth (≤ ⌈log₂ N⌉ + small constant
/// for unbalanced trees on duplicate codes); safe for any practical N.
fn propagate_aabb(nodes: &mut [BvhNode], idx: usize) -> Aabb {
    let node = nodes[idx];
    if node.is_leaf() {
        return Aabb::new(Vec3::from(node.aabb_min), Vec3::from(node.aabb_max));
    }
    let left = node.left as usize;
    let right = node.right_child() as usize;
    let left_aabb = propagate_aabb(nodes, left);
    let right_aabb = propagate_aabb(nodes, right);
    let combined = left_aabb.union(&right_aabb);
    nodes[idx].aabb_min = combined.min.into();
    nodes[idx].aabb_max = combined.max.into();
    combined
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
}
