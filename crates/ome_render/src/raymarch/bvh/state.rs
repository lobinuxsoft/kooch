//! [`BvhState`] — raymarch wrapper around the engine-shared
//! [`SharedBvhState`].
//!
//! Holds the renderer's view of the multi-consumer GPU BVH plus the
//! parallel raymarch-only double-buffers (see [`super::slots`]) for
//! `SdfPrimitive[]` (binding 1) and `RaymarchPayload[]` (binding 5).
//! Every BVH-side concern — nodes / sorted_indices / leaf_aabbs /
//! capacity growth — is delegated to [`SharedBvhState`]; only the
//! raymarch-side per-slot uploads live here.
//!
//! Both side double-buffers ride the orchestrator's
//! [`BuildToken::attach_payload`]: their captured `Vec`s fire
//! atomically on swap success and are dropped without running on swap
//! failure, so each slot's `(BVH, leaf_aabbs, payloads, primitives)`
//! tuple is consistent with itself by construction. That's the
//! lockstep #356 needs — see the slots module docstring for the full
//! contract.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use ome_bvh::{Aabb, BvhBuildError, LeafAabb, SharedBvhState, SwapInfo};

use super::slots::{
    INITIAL_SIDE_CAPACITY, PayloadSlot, PrimitiveSlot, attach_payload_upload,
    attach_primitive_upload,
};
use crate::raymarch::instance::{RaymarchPayload, SdfPrimitive};

/// Raymarch-side BVH state. Wraps [`SharedBvhState`] and the parallel
/// raymarch payload + primitive double-buffers. Bound to the renderer
/// as a sub-resource of `RayMarchRenderer`.
pub struct BvhState {
    shared: SharedBvhState,
    payload_slots: [PayloadSlot; 2],
    primitive_slots: [PrimitiveSlot; 2],
}

impl BvhState {
    /// Build the GPU compute infrastructure + initial empty slots.
    /// `pipeline_cache` is forwarded to the LBVH builder — pass the
    /// engine's shared `wgpu::PipelineCache` to amortise compile time.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
        Self {
            shared: SharedBvhState::new(device, queue, pipeline_cache),
            payload_slots: [
                PayloadSlot::new(device, INITIAL_SIDE_CAPACITY),
                PayloadSlot::new(device, INITIAL_SIDE_CAPACITY),
            ],
            primitive_slots: [
                PrimitiveSlot::new(device, INITIAL_SIDE_CAPACITY),
                PrimitiveSlot::new(device, INITIAL_SIDE_CAPACITY),
            ],
        }
    }

    /// Borrow the BVH-nodes buffer for the currently-active slot.
    pub fn current_nodes(&self) -> &wgpu::Buffer {
        self.shared.current_nodes()
    }

    /// Borrow the sorted-indices buffer for the currently-active slot.
    pub fn current_sorted_indices(&self) -> &wgpu::Buffer {
        self.shared.current_sorted_indices()
    }

    /// Borrow the leaf-AABB buffer for the currently-active slot.
    pub fn current_leaf_aabbs(&self) -> &wgpu::Buffer {
        self.shared.current_leaf_aabbs()
    }

    /// Borrow the raymarch-only payload buffer for the currently-active
    /// slot. Only the raymarch fragment shader needs this binding —
    /// physics broadphase and frustum culling skip it.
    pub fn current_raymarch_payloads(&self) -> &wgpu::Buffer {
        let idx = self.shared.current_slot_index() as usize;
        &self.payload_slots[idx].buffer
    }

    /// Borrow the raymarch-only `SdfPrimitive[]` buffer for the
    /// currently-active slot. Slot-rotated alongside `leaf_aabbs` so
    /// the BVH cull and the SDF evaluation always agree on the scene
    /// state; see #356 for why this matters.
    pub fn current_primitives(&self) -> &wgpu::Buffer {
        let idx = self.shared.current_slot_index() as usize;
        &self.primitive_slots[idx].buffer
    }

    /// Number of valid primitives in the currently-active slot. `0`
    /// before any build has resolved.
    pub fn current_n(&self) -> u32 {
        self.shared.current_n()
    }

    /// Compute a stable hash of `(items + leaf_aabbs + raymarch_payloads
    /// + primitives)` so callers can detect whether the scene changed
    /// since the last build. Hashing the side payloads alongside the
    /// items + leaves is load-bearing: a smoothness or rotation change
    /// with no AABB delta must still trigger a re-upload, otherwise the
    /// bound slot keeps stale data and the rendered output silently
    /// drifts.
    pub fn hash_scene(
        items: &[(u32, Aabb)],
        leaf_aabbs: &[LeafAabb],
        raymarch_payloads: &[RaymarchPayload],
        primitives: &[SdfPrimitive],
    ) -> u64 {
        let mut h = DefaultHasher::new();
        items.len().hash(&mut h);
        for (id, a) in items {
            id.hash(&mut h);
            for c in a.min.to_array().iter().chain(a.max.to_array().iter()) {
                c.to_bits().hash(&mut h);
            }
        }
        leaf_aabbs.len().hash(&mut h);
        for la in leaf_aabbs {
            la.flags.hash(&mut h);
            la.entity_id.hash(&mut h);
        }
        raymarch_payloads.len().hash(&mut h);
        for rp in raymarch_payloads {
            rp.smoothness.to_bits().hash(&mut h);
        }
        primitives.len().hash(&mut h);
        for p in primitives {
            // Cover every byte the fragment shader reads — a pure
            // rotation or a type-tag swap with no AABB delta must
            // still invalidate the cached slot.
            p.type_tag.hash(&mut h);
            for c in p
                .position
                .iter()
                .chain(p.rotation.iter())
                .chain(p.scale.iter())
                .chain(p.params.iter())
            {
                c.to_bits().hash(&mut h);
            }
        }
        h.finish()
    }

    /// Force a full rebuild on dirty. Production frame loops use
    /// [`Self::kick_auto_if_dirty`] instead — this entry point stays
    /// for tests / tooling that need a deterministic full build.
    ///
    /// `items`, `leaf_aabbs`, `raymarch_payloads`, and `primitives`
    /// must all be the same length and ordering: position `i` is the
    /// same primitive across every slice.
    ///
    /// At most one build is in flight at a time. If a previous build
    /// has not yet been polled to completion, this is a no-op
    /// regardless of the dirty state — the caller should drive
    /// [`Self::poll_swap`] every frame to keep the pipeline moving.
    #[allow(dead_code)]
    pub fn kick_if_dirty(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        items: Vec<(u32, Aabb)>,
        leaf_aabbs: Vec<LeafAabb>,
        raymarch_payloads: Vec<RaymarchPayload>,
        primitives: Vec<SdfPrimitive>,
    ) -> bool {
        debug_assert_aligned(&items, &leaf_aabbs, &raymarch_payloads, &primitives);

        let scene_hash = Self::hash_scene(&items, &leaf_aabbs, &raymarch_payloads, &primitives);
        let Some(mut token) = self.shared.kick(device, queue, items, leaf_aabbs, scene_hash) else {
            return false;
        };
        attach_payload_upload(
            &mut self.payload_slots,
            device,
            &mut token,
            raymarch_payloads,
        );
        attach_primitive_upload(&mut self.primitive_slots, device, &mut token, primitives);
        true
    }

    /// Unified rebuild-vs-refit entry point. Same dirty-hash gate as
    /// [`Self::kick_if_dirty`], but the orchestrator picks between
    /// full build and refit fast-path internally based on the
    /// [`ome_bvh::should_refit`] heuristic. Returns `true` when a kick
    /// of either kind was committed.
    ///
    /// `move_threshold_ratio` and `change_threshold_pct` forward to
    /// the heuristic — PR-5 plan defaults are `0.25` and `10.0`. The
    /// renderer uses these for the production frame loop; tests that
    /// want to force a specific path call [`Self::kick_if_dirty`] or
    /// [`Self::kick_refit_if_dirty`] directly.
    pub fn kick_auto_if_dirty(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        items: Vec<(u32, Aabb)>,
        leaf_aabbs: Vec<LeafAabb>,
        raymarch_payloads: Vec<RaymarchPayload>,
        primitives: Vec<SdfPrimitive>,
        move_threshold_ratio: f32,
        change_threshold_pct: f32,
    ) -> bool {
        debug_assert_aligned(&items, &leaf_aabbs, &raymarch_payloads, &primitives);

        let scene_hash = Self::hash_scene(&items, &leaf_aabbs, &raymarch_payloads, &primitives);
        let Some(mut token) = self.shared.kick_auto(
            device,
            queue,
            items,
            leaf_aabbs,
            scene_hash,
            move_threshold_ratio,
            change_threshold_pct,
        ) else {
            return false;
        };
        attach_payload_upload(
            &mut self.payload_slots,
            device,
            &mut token,
            raymarch_payloads,
        );
        attach_primitive_upload(&mut self.primitive_slots, device, &mut token, primitives);
        true
    }

    /// Refit fast path: rewrite leaves with new AABBs over the
    /// existing topology. Returns `true` when a refit was kicked.
    ///
    /// Suppressed under the same conditions as
    /// [`SharedBvhState::kick_refit`]: a previous build / refit in
    /// flight, the scene hash unchanged, or the cardinality differing
    /// from the previous build. Callers that suspect cardinality
    /// changed (insertions / removals) should call
    /// [`Self::kick_if_dirty`] instead.
    ///
    /// Caller invariants — silent corruption otherwise:
    /// - `items[i].0` is at the same array position as in the
    ///   immediately-preceding successful build / refit.
    /// - `leaf_aabbs[i]`, `raymarch_payloads[i]`, and `primitives[i]`
    ///   align 1:1 with `items[i]`.
    #[allow(dead_code)]
    pub fn kick_refit_if_dirty(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        items: Vec<(u32, Aabb)>,
        leaf_aabbs: Vec<LeafAabb>,
        raymarch_payloads: Vec<RaymarchPayload>,
        primitives: Vec<SdfPrimitive>,
    ) -> bool {
        debug_assert_aligned(&items, &leaf_aabbs, &raymarch_payloads, &primitives);

        let scene_hash = Self::hash_scene(&items, &leaf_aabbs, &raymarch_payloads, &primitives);
        let Some(mut token) =
            self.shared
                .kick_refit(device, queue, items, leaf_aabbs, scene_hash)
        else {
            return false;
        };
        attach_payload_upload(
            &mut self.payload_slots,
            device,
            &mut token,
            raymarch_payloads,
        );
        attach_primitive_upload(&mut self.primitive_slots, device, &mut token, primitives);
        true
    }

    /// Drive the in-flight build forward. Must be called once per
    /// frame. Returns the build outcome on the frame the swap happens:
    ///
    /// - `None` — no pending build, or pending build still in flight.
    /// - `Some(Ok(()))` — pending build resolved; the orchestrator
    ///   copied nodes / sorted_indices / leaf_aabbs to the target
    ///   slot, the attached payload + primitive uploaders fired onto
    ///   the same slot, and `current_slot` flipped.
    /// - `Some(Err(_))` — build failed; pending dropped without
    ///   touching the slots; the captured payload `Vec`s were dropped
    ///   with the uploader closures. The renderer keeps using the
    ///   previous slot's data until the next successful build.
    pub fn poll_swap(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Option<Result<(), BvhBuildError>> {
        match self.shared.poll_swap(device, queue)? {
            Err(e) => Some(Err(e)),
            Ok(SwapInfo { .. }) => Some(Ok(())),
        }
    }
}

#[track_caller]
fn debug_assert_aligned(
    items: &[(u32, Aabb)],
    leaf_aabbs: &[LeafAabb],
    raymarch_payloads: &[RaymarchPayload],
    primitives: &[SdfPrimitive],
) {
    debug_assert_eq!(
        items.len(),
        leaf_aabbs.len(),
        "items and leaf_aabbs must align 1:1 — one entry per primitive",
    );
    debug_assert_eq!(
        items.len(),
        raymarch_payloads.len(),
        "items and raymarch_payloads must align 1:1 — one entry per primitive",
    );
    debug_assert_eq!(
        items.len(),
        primitives.len(),
        "items and primitives must align 1:1 — one entry per primitive",
    );
}
