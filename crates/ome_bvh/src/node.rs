//! [`BvhNode`] — flat-array layout shared by CPU build (this PR) and
//! GPU compute build (PR-3).
//!
//! Layout chosen to be std430-compatible from the day the CPU builder
//! ships, so the WGSL port (PR-3) reads the same bytes without
//! rewriting the consumer side. Single 32-byte node, both internal
//! and leaf — discriminator is `count`.

use bytemuck::{Pod, Zeroable};

/// One node of the BVH flat array. **32 bytes**, naturally aligned for
/// `std430` storage buffers (no internal padding, all members are
/// 4-byte multiples).
///
/// # Variants by `count`
///
/// - `count == 0` → **internal** node. `left_or_first` is the index of
///   the left child in the flat array; the right child is at
///   `left_or_first + 1` (binary BVH, children always allocated as a
///   contiguous pair).
/// - `count > 0` → **leaf** node. `left_or_first` is the index of the
///   first item this leaf owns in the leaves payload array; `count` is
///   the number of contiguous items.
///
/// `aabb_min` / `aabb_max` always describe the union of children
/// (internal) or the union of leaf items' bounds (leaf).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default, Debug, PartialEq)]
pub struct BvhNode {
    pub aabb_min: [f32; 3],
    pub left_or_first: u32,
    pub aabb_max: [f32; 3],
    pub count: u32,
}

impl BvhNode {
    /// Build an internal node referring to a left-child index. The
    /// right child is implicit at `left + 1`.
    pub fn internal(aabb_min: [f32; 3], aabb_max: [f32; 3], left: u32) -> Self {
        Self {
            aabb_min,
            left_or_first: left,
            aabb_max,
            count: 0,
        }
    }

    /// Build a leaf node owning `count` items starting at `first` in
    /// the leaves payload array.
    pub fn leaf(aabb_min: [f32; 3], aabb_max: [f32; 3], first: u32, count: u32) -> Self {
        debug_assert!(count > 0, "leaf nodes must own at least one item");
        Self {
            aabb_min,
            left_or_first: first,
            aabb_max,
            count,
        }
    }

    pub fn is_leaf(&self) -> bool {
        self.count > 0
    }

    pub fn is_internal(&self) -> bool {
        self.count == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_is_32_bytes_aligned_4() {
        assert_eq!(std::mem::size_of::<BvhNode>(), 32);
        assert_eq!(std::mem::align_of::<BvhNode>(), 4);
    }

    #[test]
    fn pod_zeroable() {
        // Compile-time assertion via `bytemuck::Zeroable::zeroed`.
        let z = BvhNode::zeroed();
        assert_eq!(z.aabb_min, [0.0, 0.0, 0.0]);
        assert_eq!(z.aabb_max, [0.0, 0.0, 0.0]);
        assert_eq!(z.left_or_first, 0);
        assert_eq!(z.count, 0);
    }

    #[test]
    fn internal_constructor_marks_count_zero() {
        let n = BvhNode::internal([0.0; 3], [1.0; 3], 7);
        assert_eq!(n.count, 0);
        assert_eq!(n.left_or_first, 7);
        assert!(n.is_internal());
        assert!(!n.is_leaf());
    }

    #[test]
    fn leaf_constructor_records_first_and_count() {
        let n = BvhNode::leaf([0.0; 3], [1.0; 3], 42, 3);
        assert_eq!(n.count, 3);
        assert_eq!(n.left_or_first, 42);
        assert!(n.is_leaf());
        assert!(!n.is_internal());
    }

    #[test]
    fn bytemuck_cast_round_trip() {
        let n = BvhNode::internal([1.0, 2.0, 3.0], [4.0, 5.0, 6.0], 10);
        let bytes = bytemuck::bytes_of(&n);
        assert_eq!(bytes.len(), 32);
        let back: &BvhNode = bytemuck::from_bytes(bytes);
        assert_eq!(*back, n);
    }
}
