//! `impl Bvh<T>` — owning constructors, slice-destination build,
//! GPU build entry point and the topology-preserving owning refit.
//! All algorithmic logic for Karras lives in `karras.rs`; AABB
//! propagation and the slice-destination refit live in `refit.rs`.

use glam::Vec3;

use crate::aabb::Aabb;
use crate::leaf::LeafAabb;
use crate::morton::MortonCode;
use crate::node::BvhNode;

use super::karras::karras_range_and_split;
use super::refit::{propagate_aabb, refit_slice_in_place};
use super::types::{BuildMeta, Bvh};

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
    ///
    /// Allocates owning `Vec`s for the result. When the destination
    /// is a pool slice (e.g. `OmeAccel::bvh_nodes_pool`), prefer
    /// [`Self::build_into`] — same algorithm, no allocation.
    pub fn build(items: Vec<(T, Aabb)>) -> Self {
        if items.is_empty() {
            return Self::empty();
        }
        let n = items.len();
        let total_nodes = 2 * n - 1;
        let mut nodes: Vec<BvhNode> = vec![BvhNode::default(); total_nodes];
        let mut leaves: Vec<T> = Vec::with_capacity(n);
        leaves.resize(n, items[0].0);
        let _ = Self::build_into(items, &mut nodes, &mut leaves);
        Self { nodes, leaves }
    }

    /// Slice-destination variant of [`Self::build`]. Writes the
    /// `2N - 1` nodes into `nodes_dst[..2N - 1]` and the leaf
    /// permutation into `leaves_dst[..N]`. Both slices MUST be
    /// pre-sized to at least `2N - 1` and `N` respectively — the pool
    /// allocator in `OmeAccel` reserves them via the `FreeListPool`
    /// before calling.
    ///
    /// Returns the build metadata the caller stores into a
    /// `ChunkDescriptor` (root AABB, populated counts).
    ///
    /// Empty input is a no-op and returns
    /// `BuildMeta { node_count: 0, leaf_count: 0, root_aabb: EMPTY }`.
    /// Same Karras + propagate algorithm as [`Self::build`] —
    /// byte-identical output for the same input.
    pub fn build_into(
        items: Vec<(T, Aabb)>,
        nodes_dst: &mut [BvhNode],
        leaves_dst: &mut [T],
    ) -> BuildMeta {
        if items.is_empty() {
            return BuildMeta {
                node_count: 0,
                leaf_count: 0,
                root_aabb: Aabb::EMPTY,
            };
        }
        let n = items.len();
        let total_nodes = 2 * n - 1;
        debug_assert!(
            nodes_dst.len() >= total_nodes,
            "build_into: nodes_dst capacity {} < required {}",
            nodes_dst.len(),
            total_nodes,
        );
        debug_assert!(
            leaves_dst.len() >= n,
            "build_into: leaves_dst capacity {} < required {}",
            leaves_dst.len(),
            n,
        );

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

        // 4. Write leaves into [leaf_offset, total_nodes) and
        //    populate the leaves payload permutation.
        for (k, item) in morton_items.iter().enumerate() {
            let aabb = &item.2;
            nodes_dst[leaf_offset + k] = BvhNode::leaf(
                aabb.min.into(),
                aabb.max.into(),
                k as u32,
                1,
            );
            leaves_dst[k] = item.1;
        }

        // 5. Compute internals via Karras (skipped when N==1: no
        // internal nodes, the single leaf at nodes[0] is the root).
        if n > 1 {
            let morton: Vec<u32> = morton_items.iter().map(|(c, _, _)| c.0).collect();
            for i in 0..(n - 1) {
                let (first, last, gamma) = karras_range_and_split(&morton, i);
                let left_idx = if gamma == first {
                    (leaf_offset + gamma) as u32
                } else {
                    gamma as u32
                };
                let right_idx = if gamma + 1 == last {
                    (leaf_offset + gamma + 1) as u32
                } else {
                    (gamma + 1) as u32
                };
                nodes_dst[i] = BvhNode::internal([0.0; 3], [0.0; 3], left_idx, right_idx);
            }

            // 6. Bottom-up AABB propagation from root via post-order DFS.
            propagate_aabb(&mut nodes_dst[..total_nodes], 0);
        }

        let root_aabb = Aabb::new(
            Vec3::from(nodes_dst[0].aabb_min),
            Vec3::from(nodes_dst[0].aabb_max),
        );
        BuildMeta {
            node_count: total_nodes as u32,
            leaf_count: n as u32,
            root_aabb,
        }
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

    /// Refit AABBs over the existing topology. `leaf_aabbs[i]` is the
    /// (possibly-updated) AABB of original-input position `i`, in the
    /// same order as the items vector handed to [`Self::build`].
    /// `sorted_indices[k]` is the original-input position of the leaf
    /// at sorted position `k` (i.e. the leaf node at `nodes[(N-1) + k]`).
    /// On the GPU path this is the `values_a` permutation captured at
    /// the last successful build.
    ///
    /// Same caller invariants as [`crate::BvhGpuRefit`]:
    /// - `leaf_aabbs.len() == self.leaf_count()` — cardinality preserved.
    /// - `sorted_indices` is the permutation from the last successful
    ///   build (or chained refit). The topology must be the one the
    ///   permutation describes; a stale `sorted_indices` produces
    ///   wrong-but-plausible AABBs without panicking.
    ///
    /// Pure CPU work; O(N) leaf writes + bottom-up propagate.
    /// `OmeAccel`'s topology-preserving refit hook
    /// (`refit_chunk_slice_only`) drives this against the per-chunk
    /// CPU shadow without paying for a full `(2N-1) * 32 B` nodes
    /// readback.
    pub fn refit_in_place(&mut self, leaf_aabbs: &[LeafAabb], sorted_indices: &[u32]) {
        let n = self.leaf_count();
        refit_slice_in_place(&mut self.nodes, n, leaf_aabbs, sorted_indices);
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
