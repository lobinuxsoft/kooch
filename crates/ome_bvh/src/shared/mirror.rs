//! CPU-side mirror of the GPU BVH. Owned by [`super::SharedBvhState`]
//! and refreshed by [`super::SharedBvhState::poll_swap`] on every
//! successful build / refit swap. CPU consumers (physics broadphase,
//! debug tooling, future authoring traversals) read the mirror to run
//! [`Bvh::for_each_aabb`] and friends without paying for a separate
//! CPU build.
//!
//! The readback that produces `bvh` + `sorted_indices` was already
//! paid for by the GPU build path (`BvhGpuBuild::poll` always returns
//! the readback result; pre-S4 the orchestrator threw it away). On
//! refit the mirror is updated in place via [`Bvh::refit_in_place`]
//! using the stored permutation — no extra GPU traffic, no new
//! readback.

use crate::leaf::LeafAabb;
use crate::{Bvh, BvhGpuBuildResult};

/// CPU-side mirror of the most recent successful build / refit.
pub(super) struct CpuMirror {
    pub(super) bvh: Bvh<u32>,
    /// Permutation captured at the last full build. Reused on every
    /// subsequent refit until the next full build supplies a new one.
    pub(super) sorted_indices: Vec<u32>,
    /// Per-leaf metadata in **original input order** (matches the
    /// ordering of `items` handed to `kick` / `kick_refit`). Mirrors
    /// the GPU's `leaf_aabbs_buffer` of the currently-active slot.
    /// CPU consumers filter this by `IS_*` flags to scope their query
    /// to their consumer subset.
    pub(super) leaf_aabbs: Vec<LeafAabb>,
}

impl CpuMirror {
    /// Construct a fresh mirror from a resolved GPU build outcome —
    /// new bvh, new sorted_indices permutation, new leaf_aabbs. Topology
    /// may have changed; the previous mirror (if any) is discarded.
    pub(super) fn from_build(
        result: BvhGpuBuildResult<u32>,
        leaf_aabbs: Vec<LeafAabb>,
    ) -> Self {
        let BvhGpuBuildResult {
            bvh,
            sorted_indices,
        } = result;
        Self {
            bvh,
            sorted_indices,
            leaf_aabbs,
        }
    }

    /// Apply a refit in place. Topology preserved by the kick_refit
    /// invariant; we keep `bvh` and `sorted_indices` and re-propagate
    /// AABBs from the new `leaf_aabbs` over the existing tree.
    pub(super) fn apply_refit(&mut self, leaf_aabbs: Vec<LeafAabb>) {
        self.leaf_aabbs = leaf_aabbs;
        self.bvh
            .refit_in_place(&self.leaf_aabbs, &self.sorted_indices);
    }
}
