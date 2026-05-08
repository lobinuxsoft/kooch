//! Public data types for the LBVH: the [`Bvh<T>`] container and the
//! [`BuildMeta`] returned by the slice-destination build variant.

use crate::aabb::Aabb;
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

/// Metadata returned by the slice-destination
/// [`Bvh::build_into`] variant. Mirrors the fields a
/// `ChunkDescriptor` consumes after the BLAS write.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BuildMeta {
    /// `2N - 1` for `N >= 1`, zero for empty input.
    pub node_count: u32,
    /// Number of input leaves (= number of populated entries in
    /// `leaves_dst`).
    pub leaf_count: u32,
    /// Bounding box of the root, ready to copy into
    /// `ChunkDescriptor.{aabb_min, aabb_max}` (after envelope
    /// inflation by `max_smoothness_radius`).
    pub root_aabb: Aabb,
}
