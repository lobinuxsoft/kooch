//! [`Bvh<T>`] — generic LBVH built from a `Vec<(T, Aabb)>` via the
//! Karras 2012 parallel construction algorithm.
//!
//! The CPU build mirrors the WGSL compute build (PR-3 of #115)
//! byte-for-byte: same node layout, same indexing convention, same
//! AABB propagation order. `Bvh::build` and `Bvh::build_gpu` produce
//! identical [`BvhNode`] arrays for the same input.
//!
//! [`BvhNode`]: crate::node::BvhNode
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
//! [`BvhNode::left`]: crate::node::BvhNode::left
//! [`BvhNode::right_child`]: crate::node::BvhNode::right_child
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
//!
//! [`MortonCode`]: crate::morton::MortonCode

mod builder;
mod karras;
mod refit;
mod types;

#[cfg(test)]
mod tests;

pub use refit::refit_slice_in_place;
pub use types::{BuildMeta, Bvh};
