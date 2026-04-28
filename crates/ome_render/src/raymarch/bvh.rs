//! Double-buffered GPU BVH state for the raymarch primitive culling
//! integration (PR-4 of #115).
//!
//! Owns a [`BvhGpuBuilder`] (the reusable LBVH compute pipeline + scratch
//! buffers from PR-3) plus two output slots — `slot_a` and `slot_b` —
//! each holding a stable copy of the last completed build's outputs
//! (nodes, sorted indices, leaf AABBs). The renderer always reads from
//! `current_slot`; pending builds copy their results into the OTHER slot
//! at swap time, so the renderer never observes a half-written buffer
//! and the GPU avoids a stall on the read-after-write hazard a single
//! shared buffer would introduce.
//!
//! The stale-buffer copy adds a constant amount of GPU bandwidth per
//! build (`(2N - 1) * 32` bytes for nodes + `N * 4` for indices) — far
//! cheaper than blocking the frame pipeline.
//!
//! # Lifecycle per frame
//!
//! ```text
//!   1. (frame start) bvh_state.poll_swap(device, queue):
//!        - drives wgpu's map_async callbacks via device.poll(Poll)
//!        - if pending build resolved, copy outputs to `target_slot`,
//!          flip current_slot, drop pending.
//!   2. Caller computes the new scene-state hash.
//!   3. bvh_state.kick_if_dirty(...) — no-op if hash matches; otherwise
//!      starts a new GPU build into the FREE slot (the one not currently
//!      bound to the renderer).
//!   4. Renderer binds bvh_state.current_*() buffers in the raymarch
//!      pass — guaranteed to be the last completed build.
//! ```
//!
//! # Empty / first-frame handling
//!
//! Before any build completes, `current_n() == 0` and the bind buffers
//! are minimal placeholders. The shader's `n == 0` branch must render
//! the sky, mirroring PR-3's `Bvh::empty()` semantics.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use ome_bvh::{Aabb, BvhBuildError, BvhGpuBuild, BvhGpuBuilder, BvhNode};
use wgpu::util::DeviceExt;

use super::instance::{INITIAL_LEAF_AABB_CAPACITY, LeafAabb};

/// Initial capacity (in leaves) for the per-slot stable buffers. Grows
/// by `next_power_of_two` whenever a build exceeds it.
const INITIAL_SLOT_CAPACITY: u64 = 256;

/// Per-slot stable buffer set. The renderer binds these directly when
/// the slot is `current`. Capacity grows on demand; buffers never shrink.
struct OutputSlot {
    /// Flat tree of `BvhNode`s. Sized for `2N` to keep capacity
    /// arithmetic simple (real fill is `2N - 1`).
    nodes_buffer: wgpu::Buffer,
    /// `sorted_indices[k]` = original payload index at sorted position
    /// `k`. Length `N`.
    sorted_indices_buffer: wgpu::Buffer,
    /// Per-primitive metadata (AABB + role + smoothness). Length `N`.
    leaf_aabbs_buffer: wgpu::Buffer,
    /// Capacity in leaves (matches `LbvhBuffers::capacity`'s convention).
    capacity: u64,
    /// Number of valid leaves (= primitives) in this slot. `0` until the
    /// first build resolves into it.
    n: u32,
}

impl OutputSlot {
    fn new(device: &wgpu::Device, capacity: u64) -> Self {
        let nodes_buffer = make_nodes_buffer(device, capacity);
        let sorted_indices_buffer = make_indices_buffer(device, capacity);
        let leaf_aabbs_buffer = make_leaf_aabbs_buffer(device, capacity);
        Self {
            nodes_buffer,
            sorted_indices_buffer,
            leaf_aabbs_buffer,
            capacity,
            n: 0,
        }
    }

    fn ensure_capacity(&mut self, device: &wgpu::Device, n: u64) {
        if n <= self.capacity {
            return;
        }
        let new_cap = n.next_power_of_two().max(INITIAL_SLOT_CAPACITY);
        self.nodes_buffer = make_nodes_buffer(device, new_cap);
        self.sorted_indices_buffer = make_indices_buffer(device, new_cap);
        self.leaf_aabbs_buffer = make_leaf_aabbs_buffer(device, new_cap);
        self.capacity = new_cap;
    }
}

fn make_nodes_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("raymarch_bvh::slot::nodes"),
        size: 2 * capacity * std::mem::size_of::<BvhNode>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn make_indices_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("raymarch_bvh::slot::sorted_indices"),
        size: capacity * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn make_leaf_aabbs_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("raymarch_bvh::slot::leaf_aabbs"),
        size: capacity * std::mem::size_of::<LeafAabb>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// In-flight build awaiting GPU completion. Once
/// [`BvhGpuBuild::poll`] returns `Some(Ok(_))`, [`BvhState::poll_swap`]
/// copies the result into `target_slot` and flips `current_slot`.
struct PendingBuild {
    build: BvhGpuBuild<u32>,
    /// Slot index (0 or 1) the result will land in.
    target_slot: u8,
    /// Per-leaf metadata captured at kick time. Uploaded to the target
    /// slot's `leaf_aabbs_buffer` at swap. Held on the CPU because the
    /// LBVH builder doesn't see this metadata.
    leaf_aabbs: Vec<LeafAabb>,
    /// Number of leaves submitted to the build. Used at swap time to
    /// drive the copy lengths.
    n: u32,
}

/// Double-buffered BVH GPU state. Plug-in struct held as a sub-resource
/// of the raymarch renderer.
pub struct BvhState {
    pub(super) builder: BvhGpuBuilder,
    slot_a: OutputSlot,
    slot_b: OutputSlot,
    /// `0` → renderer reads `slot_a`, builds target `slot_b`. `1` → mirror.
    current_slot: u8,
    pending: Option<PendingBuild>,
    /// Hash of the last successfully kicked scene state (primitives
    /// bytes + per-leaf metadata). Used to skip redundant builds when
    /// the scene hasn't changed between frames.
    dirty_hash: Option<u64>,
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
        let builder = BvhGpuBuilder::new(device, queue, pipeline_cache);
        let initial = INITIAL_LEAF_AABB_CAPACITY.max(INITIAL_SLOT_CAPACITY);
        Self {
            builder,
            slot_a: OutputSlot::new(device, initial),
            slot_b: OutputSlot::new(device, initial),
            current_slot: 0,
            pending: None,
            dirty_hash: None,
        }
    }

    /// Borrow the slot the renderer should bind this frame.
    pub fn current_nodes(&self) -> &wgpu::Buffer {
        &self.slot(self.current_slot).nodes_buffer
    }

    pub fn current_sorted_indices(&self) -> &wgpu::Buffer {
        &self.slot(self.current_slot).sorted_indices_buffer
    }

    pub fn current_leaf_aabbs(&self) -> &wgpu::Buffer {
        &self.slot(self.current_slot).leaf_aabbs_buffer
    }

    /// Number of valid primitives in the currently-bound slot. `0`
    /// before any build has resolved.
    pub fn current_n(&self) -> u32 {
        self.slot(self.current_slot).n
    }

    /// Returns `true` when there is no in-flight build.
    pub fn is_idle(&self) -> bool {
        self.pending.is_none()
    }

    fn slot(&self, idx: u8) -> &OutputSlot {
        match idx {
            0 => &self.slot_a,
            _ => &self.slot_b,
        }
    }

    fn slot_mut(&mut self, idx: u8) -> &mut OutputSlot {
        match idx {
            0 => &mut self.slot_a,
            _ => &mut self.slot_b,
        }
    }

    /// Compute a stable hash of `(items + leaf_aabbs)` so callers can
    /// detect whether the scene changed since the last build.
    /// Exposed publicly so the update path can hash before paying the
    /// cost of formatting payload Vecs.
    pub fn hash_scene(items: &[(u32, Aabb)], leaf_aabbs: &[LeafAabb]) -> u64 {
        let mut h = DefaultHasher::new();
        items.len().hash(&mut h);
        for (id, a) in items {
            id.hash(&mut h);
            // Aabb's f32 fields don't implement Hash directly; project
            // through their bit patterns.
            for c in a.min.to_array().iter().chain(a.max.to_array().iter()) {
                c.to_bits().hash(&mut h);
            }
        }
        leaf_aabbs.len().hash(&mut h);
        for la in leaf_aabbs {
            la.role.hash(&mut h);
            la.smoothness.to_bits().hash(&mut h);
        }
        h.finish()
    }

    /// Start a new GPU build if the scene's hash changed since the last
    /// kick. Returns `true` when a new build was kicked.
    ///
    /// `items` is the BVH builder input; `leaf_aabbs` is the parallel
    /// per-leaf metadata that the WGSL traversal will consume. The two
    /// slices must be the same length and ordering.
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
    ) -> bool {
        debug_assert_eq!(
            items.len(),
            leaf_aabbs.len(),
            "items and leaf_aabbs must align 1:1 — one entry per primitive",
        );
        if self.pending.is_some() {
            return false;
        }
        let hash = Self::hash_scene(&items, &leaf_aabbs);
        if Some(hash) == self.dirty_hash {
            return false;
        }
        let n = items.len() as u32;
        // Free slot is the one NOT currently bound to the renderer.
        let target_slot = self.current_slot ^ 1;

        // Make sure the target slot can hold the result. Always grow to
        // at least 1 to keep the buffers valid even for empty scenes.
        let needed = (n as u64).max(1);
        self.slot_mut(target_slot).ensure_capacity(device, needed);

        let build = ome_bvh::Bvh::<u32>::build_gpu(&mut self.builder, device, queue, items);
        self.pending = Some(PendingBuild {
            build,
            target_slot,
            leaf_aabbs,
            n,
        });
        self.dirty_hash = Some(hash);
        true
    }

    /// Drive the in-flight build forward. Must be called once per
    /// frame. Returns the build outcome on the frame the swap happens:
    ///
    /// - `None` — no pending build, or pending build still in flight.
    /// - `Some(Ok(()))` — pending build resolved; result copied into
    ///   the target slot; `current_slot` flipped.
    /// - `Some(Err(_))` — build failed; pending dropped without
    ///   touching the slots. The renderer keeps using the previous
    ///   slot's data until the next successful build.
    pub fn poll_swap(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Option<Result<(), BvhBuildError>> {
        device.poll(wgpu::PollType::Poll).ok()?;
        let Some(pending) = self.pending.as_mut() else {
            return None;
        };
        let outcome = pending.build.poll(device)?;
        // Take ownership now — we'll either commit or drop based on the
        // outcome.
        let pending = self.pending.take().expect("just observed Some above");

        let bvh = match outcome {
            Ok(b) => b,
            Err(e) => return Some(Err(e)),
        };

        // Copy the GPU build's output (still living in the builder's
        // internal scratch buffers) into the target slot's stable
        // buffers. Empty builds (n == 0) leave the slot's `n = 0`,
        // signalling "no primitives" to the renderer.
        let n = pending.n;
        let target_slot = pending.target_slot;
        if n > 0 {
            let total_nodes = (2 * n - 1) as u64;
            let nodes_bytes = total_nodes * std::mem::size_of::<BvhNode>() as u64;
            let indices_bytes = (n as u64) * 4;
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("raymarch_bvh::poll_swap_copy_encoder"),
            });
            encoder.copy_buffer_to_buffer(
                self.builder.nodes_buffer(),
                0,
                &self.slot(target_slot).nodes_buffer,
                0,
                nodes_bytes,
            );
            encoder.copy_buffer_to_buffer(
                self.builder.sorted_indices_buffer(),
                0,
                &self.slot(target_slot).sorted_indices_buffer,
                0,
                indices_bytes,
            );
            queue.submit(std::iter::once(encoder.finish()));
            queue.write_buffer(
                &self.slot(target_slot).leaf_aabbs_buffer,
                0,
                bytemuck::cast_slice(&pending.leaf_aabbs),
            );
        }
        self.slot_mut(target_slot).n = n;
        self.current_slot = target_slot;
        // Drop the build handle (and `bvh` Vec<BvhNode> we don't need)
        // explicitly to make the lifetime obvious.
        drop(bvh);
        Some(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use ome_bvh::Aabb;

    fn dummy_leaf(role: u32, smoothness: f32) -> LeafAabb {
        LeafAabb {
            aabb_min: [0.0; 3],
            role,
            aabb_max: [1.0; 3],
            smoothness,
        }
    }

    #[test]
    fn hash_is_stable_for_identical_inputs() {
        let items = vec![
            (0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE)),
            (1u32, Aabb::from_centre(Vec3::splat(5.0), Vec3::ONE)),
        ];
        let leaves = vec![dummy_leaf(0, 0.0), dummy_leaf(1, 0.5)];
        let h1 = BvhState::hash_scene(&items, &leaves);
        let h2 = BvhState::hash_scene(&items, &leaves);
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_changes_when_aabb_changes() {
        let items_a = vec![(0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE))];
        let items_b = vec![(0u32, Aabb::from_centre(Vec3::X, Vec3::ONE))];
        let leaves = vec![dummy_leaf(0, 0.0)];
        assert_ne!(
            BvhState::hash_scene(&items_a, &leaves),
            BvhState::hash_scene(&items_b, &leaves),
        );
    }

    #[test]
    fn hash_changes_when_role_changes() {
        let items = vec![(0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE))];
        let leaves_add = vec![dummy_leaf(0, 0.5)];
        let leaves_int = vec![dummy_leaf(1, 0.5)];
        assert_ne!(
            BvhState::hash_scene(&items, &leaves_add),
            BvhState::hash_scene(&items, &leaves_int),
        );
    }

    #[test]
    fn hash_changes_when_smoothness_changes() {
        let items = vec![(0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE))];
        let leaves_lo = vec![dummy_leaf(0, 0.1)];
        let leaves_hi = vec![dummy_leaf(0, 0.5)];
        assert_ne!(
            BvhState::hash_scene(&items, &leaves_lo),
            BvhState::hash_scene(&items, &leaves_hi),
        );
    }

    #[test]
    fn hash_changes_when_count_changes() {
        let leaves = vec![dummy_leaf(0, 0.0)];
        let items_one = vec![(0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE))];
        let items_two = vec![
            (0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE)),
            (1u32, Aabb::from_centre(Vec3::X, Vec3::ONE)),
        ];
        let leaves_two = vec![dummy_leaf(0, 0.0), dummy_leaf(0, 0.0)];
        assert_ne!(
            BvhState::hash_scene(&items_one, &leaves),
            BvhState::hash_scene(&items_two, &leaves_two),
        );
    }
}
