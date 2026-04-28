//! [`BvhState`] — raymarch wrapper around the engine-shared
//! [`SharedBvhState`].
//!
//! Holds the renderer's view of the multi-consumer GPU BVH plus a
//! parallel double-buffer for the raymarch-only [`RaymarchPayload`]
//! storage buffer (binding 5 of the raymarch fragment shader). Every
//! BVH-side concern — nodes / sorted_indices / leaf_aabbs / capacity
//! growth — is delegated to [`SharedBvhState`]; only the raymarch
//! side payload lives here.
//!
//! The payload double-buffer mirrors the shared BVH's swap: when
//! [`SharedBvhState::poll_swap`] reports a [`SwapInfo`] the renderer
//! uploads the captured `RaymarchPayload[]` into the same slot index.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use ome_bvh::{Aabb, BvhBuildError, LeafAabb, SharedBvhState, SwapInfo};

use crate::raymarch::instance::RaymarchPayload;

/// Initial capacity (in primitives) for the raymarch payload double-
/// buffer. Tracks `INITIAL_SLOT_CAPACITY` in `ome_bvh::shared` so
/// growth events line up across the two double-buffers.
const INITIAL_PAYLOAD_CAPACITY: u64 = 256;

/// Per-slot raymarch-only side buffer. Mirrors the shared BVH's slot
/// rotation; bound by the raymarch fragment shader at binding 5.
struct PayloadSlot {
    buffer: wgpu::Buffer,
    capacity: u64,
}

impl PayloadSlot {
    fn new(device: &wgpu::Device, capacity: u64) -> Self {
        Self {
            buffer: make_payload_buffer(device, capacity),
            capacity,
        }
    }

    fn ensure_capacity(&mut self, device: &wgpu::Device, n: u64) {
        if n <= self.capacity {
            return;
        }
        let new_cap = n.next_power_of_two().max(INITIAL_PAYLOAD_CAPACITY);
        self.buffer = make_payload_buffer(device, new_cap);
        self.capacity = new_cap;
    }
}

fn make_payload_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("raymarch_bvh::payload_slot"),
        size: capacity * std::mem::size_of::<RaymarchPayload>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Per-build raymarch payload captured at kick time. Drained on the
/// matching [`SwapInfo`] and uploaded to the swapped-in slot.
struct PendingPayload {
    raymarch_payloads: Vec<RaymarchPayload>,
}

/// Raymarch-side BVH state. Wraps [`SharedBvhState`] and the parallel
/// raymarch payload double-buffer. Bound to the renderer as a sub-
/// resource of `RayMarchRenderer`.
pub struct BvhState {
    shared: SharedBvhState,
    payload_slots: [PayloadSlot; 2],
    pending_payload: Option<PendingPayload>,
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
                PayloadSlot::new(device, INITIAL_PAYLOAD_CAPACITY),
                PayloadSlot::new(device, INITIAL_PAYLOAD_CAPACITY),
            ],
            pending_payload: None,
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

    /// Number of valid primitives in the currently-active slot. `0`
    /// before any build has resolved.
    pub fn current_n(&self) -> u32 {
        self.shared.current_n()
    }

    /// Compute a stable hash of `(items + leaf_aabbs + raymarch_payloads)`
    /// so callers can detect whether the scene changed since the last
    /// build. Hashing the raymarch payload alongside the items + leaves
    /// is load-bearing: a smoothness change with no AABB / flag delta
    /// must still trigger a re-upload, otherwise the bound slot keeps
    /// stale `k` and the smooth blend silently drifts.
    pub fn hash_scene(
        items: &[(u32, Aabb)],
        leaf_aabbs: &[LeafAabb],
        raymarch_payloads: &[RaymarchPayload],
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
        h.finish()
    }

    /// Start a new GPU build if the scene's hash changed since the last
    /// kick. Returns `true` when a new build was kicked.
    ///
    /// `items` is the BVH builder input; `leaf_aabbs` is the parallel
    /// per-leaf metadata bound by every consumer; `raymarch_payloads`
    /// is the raymarch-only metadata bound by the fragment shader.
    /// All three slices must be the same length and ordering.
    ///
    /// At most one build is in flight at a time. If a previous build
    /// has not yet been polled to completion, this is a no-op
    /// regardless of the dirty state — the caller should drive
    /// [`Self::poll_swap`] every frame to keep the pipeline moving.
    pub fn kick_if_dirty(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        items: Vec<(u32, Aabb)>,
        leaf_aabbs: Vec<LeafAabb>,
        raymarch_payloads: Vec<RaymarchPayload>,
    ) -> bool {
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

        // Grow the target payload slot up front so the SharedBvhState
        // and the payload buffer reach the same capacity in lockstep.
        let n = items.len() as u32;
        let target_slot = self.shared.current_slot_index() ^ 1;
        let needed = (n as u64).max(1);
        self.payload_slots[target_slot as usize].ensure_capacity(device, needed);

        let scene_hash = Self::hash_scene(&items, &leaf_aabbs, &raymarch_payloads);
        let kicked = self
            .shared
            .kick(device, queue, items, leaf_aabbs, scene_hash);
        if kicked {
            self.pending_payload = Some(PendingPayload { raymarch_payloads });
        }
        kicked
    }

    /// Refit fast path: rewrite leaves with new AABBs over the
    /// existing topology. Returns `true` when a refit was kicked.
    ///
    /// `#[allow(dead_code)]`: the consumer wiring lives in S6 of
    /// PR-5 (the refit-vs-rebuild policy). The method is part of
    /// the public-facing API the orchestrator will pick up there.
    #[allow(dead_code)]
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
    /// - `leaf_aabbs[i]` and `raymarch_payloads[i]` align 1:1 with
    ///   `items[i]`.
    pub fn kick_refit_if_dirty(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        items: Vec<(u32, Aabb)>,
        leaf_aabbs: Vec<LeafAabb>,
        raymarch_payloads: Vec<RaymarchPayload>,
    ) -> bool {
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

        let n = items.len() as u32;
        let target_slot = self.shared.current_slot_index() ^ 1;
        let needed = (n as u64).max(1);
        self.payload_slots[target_slot as usize].ensure_capacity(device, needed);

        let scene_hash = Self::hash_scene(&items, &leaf_aabbs, &raymarch_payloads);
        let kicked = self
            .shared
            .kick_refit(device, queue, items, leaf_aabbs, scene_hash);
        if kicked {
            self.pending_payload = Some(PendingPayload { raymarch_payloads });
        }
        kicked
    }

    /// Drive the in-flight build forward. Must be called once per
    /// frame. Returns the build outcome on the frame the swap happens:
    ///
    /// - `None` — no pending build, or pending build still in flight.
    /// - `Some(Ok(()))` — pending build resolved; result copied into
    ///   the target slot; raymarch payload uploaded onto the same slot;
    ///   `current_slot` flipped.
    /// - `Some(Err(_))` — build failed; pending dropped without
    ///   touching the slots. The renderer keeps using the previous
    ///   slot's data until the next successful build.
    pub fn poll_swap(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Option<Result<(), BvhBuildError>> {
        match self.shared.poll_swap(device, queue)? {
            Err(e) => {
                // Build failed; drop the captured payload — there is
                // no slot to upload it to.
                self.pending_payload = None;
                Some(Err(e))
            }
            Ok(SwapInfo { target_slot, n }) => {
                if let Some(pending) = self.pending_payload.take()
                    && n > 0
                {
                    queue.write_buffer(
                        &self.payload_slots[target_slot as usize].buffer,
                        0,
                        bytemuck::cast_slice(&pending.raymarch_payloads),
                    );
                }
                Some(Ok(()))
            }
        }
    }
}
