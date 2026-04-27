//! [`BvhNode`] — flat-array layout shared by CPU build and GPU compute
//! build (PR-3 of #115).
//!
//! Layout chosen to be std430-compatible from day one so the WGSL port
//! reads the same bytes without rewriting the consumer side. Single
//! 32-byte node, both internal and leaf — discriminator is the high
//! bit of `right_or_count`.

use bytemuck::{Pod, Zeroable};

/// High bit of `right_or_count`. Set → leaf, clear → internal.
pub const BVH_LEAF_FLAG: u32 = 1u32 << 31;

/// Mask isolating the low 31 bits of `right_or_count` (the value
/// payload — either `count` for leaves or `right_child_idx` for
/// internals).
pub const BVH_VALUE_MASK: u32 = 0x7FFF_FFFF;

/// One node of the BVH flat array. **32 bytes**, naturally aligned for
/// `std430` storage buffers (no internal padding, all members are
/// 4-byte multiples).
///
/// # Variants
///
/// Discriminator is the high bit of [`right_or_count`]:
///
/// - High bit clear → **internal** node. [`left`] is the left child
///   index in the flat array; the low 31 bits of [`right_or_count`]
///   are the right child index. Children may be at arbitrary
///   positions (not necessarily contiguous).
/// - High bit set → **leaf** node. [`left`] is the index of the first
///   item this leaf owns in the leaves payload array; the low 31 bits
///   of [`right_or_count`] are the number of contiguous items.
///
/// `aabb_min` / `aabb_max` always describe the union of children
/// (internal) or the union of leaf items' bounds (leaf).
///
/// [`left`]: Self::left
/// [`right_or_count`]: Self::right_or_count
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default, Debug, PartialEq)]
pub struct BvhNode {
    pub aabb_min: [f32; 3],
    pub left: u32,
    pub aabb_max: [f32; 3],
    pub right_or_count: u32,
}

impl BvhNode {
    /// Build an internal node referring to explicit left and right
    /// child indices. Children may be at any position in the flat
    /// array (Karras layout — they are NOT necessarily contiguous).
    pub fn internal(
        aabb_min: [f32; 3],
        aabb_max: [f32; 3],
        left: u32,
        right: u32,
    ) -> Self {
        debug_assert!(
            right & BVH_LEAF_FLAG == 0,
            "right child index must fit in 31 bits"
        );
        Self {
            aabb_min,
            left,
            aabb_max,
            right_or_count: right,
        }
    }

    /// Build a leaf node owning `count` items starting at `first` in
    /// the leaves payload array.
    pub fn leaf(aabb_min: [f32; 3], aabb_max: [f32; 3], first: u32, count: u32) -> Self {
        debug_assert!(count > 0, "leaf nodes must own at least one item");
        debug_assert!(
            count & BVH_LEAF_FLAG == 0,
            "leaf count must fit in 31 bits"
        );
        Self {
            aabb_min,
            left: first,
            aabb_max,
            right_or_count: count | BVH_LEAF_FLAG,
        }
    }

    pub fn is_leaf(&self) -> bool {
        self.right_or_count & BVH_LEAF_FLAG != 0
    }

    pub fn is_internal(&self) -> bool {
        !self.is_leaf()
    }

    /// Number of items owned by this leaf. Panics in debug if called
    /// on an internal node.
    pub fn count(&self) -> u32 {
        debug_assert!(self.is_leaf());
        self.right_or_count & BVH_VALUE_MASK
    }

    /// Right child index. Panics in debug if called on a leaf.
    pub fn right_child(&self) -> u32 {
        debug_assert!(self.is_internal());
        self.right_or_count & BVH_VALUE_MASK
    }

    /// First leaf-payload index. Panics in debug if called on an
    /// internal node.
    pub fn first_leaf(&self) -> u32 {
        debug_assert!(self.is_leaf());
        self.left
    }

    /// Left child index. Panics in debug if called on a leaf.
    pub fn left_child(&self) -> u32 {
        debug_assert!(self.is_internal());
        self.left
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
        let z = BvhNode::zeroed();
        assert_eq!(z.aabb_min, [0.0, 0.0, 0.0]);
        assert_eq!(z.aabb_max, [0.0, 0.0, 0.0]);
        assert_eq!(z.left, 0);
        assert_eq!(z.right_or_count, 0);
        // Zeroed node is internal (high bit clear).
        assert!(z.is_internal());
    }

    #[test]
    fn internal_constructor_has_clear_flag_bit() {
        let n = BvhNode::internal([0.0; 3], [1.0; 3], 7, 8);
        assert_eq!(n.left, 7);
        assert_eq!(n.right_child(), 8);
        assert!(n.is_internal());
        assert!(!n.is_leaf());
        assert_eq!(n.right_or_count & BVH_LEAF_FLAG, 0);
    }

    #[test]
    fn leaf_constructor_records_first_count_and_flag_bit() {
        let n = BvhNode::leaf([0.0; 3], [1.0; 3], 42, 3);
        assert_eq!(n.first_leaf(), 42);
        assert_eq!(n.count(), 3);
        assert!(n.is_leaf());
        assert!(!n.is_internal());
        assert_ne!(n.right_or_count & BVH_LEAF_FLAG, 0);
    }

    #[test]
    fn bytemuck_cast_round_trip() {
        let n = BvhNode::internal([1.0, 2.0, 3.0], [4.0, 5.0, 6.0], 10, 11);
        let bytes = bytemuck::bytes_of(&n);
        assert_eq!(bytes.len(), 32);
        let back: &BvhNode = bytemuck::from_bytes(bytes);
        assert_eq!(*back, n);
    }

    #[test]
    fn arbitrary_non_contiguous_children() {
        // Karras layout: left and right may be far apart (mixed
        // internal/leaf case, e.g. left=internal idx 1, right=leaf
        // idx 8 in a 5-leaf tree).
        let n = BvhNode::internal([0.0; 3], [1.0; 3], 1, 8);
        assert_eq!(n.left_child(), 1);
        assert_eq!(n.right_child(), 8);
        assert_eq!(n.right_child() - n.left_child(), 7);
    }

    #[test]
    fn count_31_bits_max() {
        // Max representable count = 2^31 - 1.
        let n = BvhNode::leaf([0.0; 3], [1.0; 3], 0, BVH_VALUE_MASK);
        assert_eq!(n.count(), BVH_VALUE_MASK);
        assert!(n.is_leaf());
    }
}
