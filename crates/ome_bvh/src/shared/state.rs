//! [`SharedBvhState`] — the orchestrator that ties the
//! [`crate::BvhGpuBuilder`] together with the double-buffered slots
//! and the kick / refit / swap lifecycle.
//!
//! # Type-state contract
//!
//! [`SharedBvhState::kick`] / [`SharedBvhState::kick_refit`] return
//! `Option<BuildToken<'_>>`. A `Some(token)` is the *only* way for a
//! consumer to learn that a kick was actually committed and to attach
//! a side-payload uploader for the upcoming swap. A `None` means the
//! kick was suppressed (dirty hash matched, a previous build is still
//! in flight, or the refit cardinality check failed); the consumer
//! holds nothing, mutates nothing, and the side-payload state stays
//! in lockstep with the orchestrator's pending by construction.
//!
//! The token is opaque: it cannot be constructed by the consumer, only
//! by the orchestrator. While a token lives the orchestrator's
//! pending is guaranteed populated, so [`BuildToken::attach_payload`]
//! never has to defend against a torn invariant.
//!
//! # CPU mirror
//!
//! The GPU build's `(Bvh<u32>, sorted_indices)` readback was already
//! paid for by [`BvhGpuBuild::poll`] (used to permute payloads). S4
//! repurposes that readback into a `cpu_mirror` so CPU consumers like
//! `ome_physics::broadphase` can run [`Bvh::for_each_aabb`] without a
//! second build. Refit updates the mirror in place via
//! [`Bvh::refit_in_place`].

use crate::leaf::LeafAabb;
use crate::{Aabb, Bvh, BvhBuildError, BvhGpuBuilder, BvhNode};

use super::mirror::CpuMirror;
use super::pending::{BuildToken, Pending, PendingKind, PollOutcome, SwapInfo};
use super::slot::{INITIAL_SLOT_CAPACITY, OutputSlot};

/// Multi-consumer double-buffered GPU BVH state. Held as a single
/// resource shared by every BVH consumer in the engine.
pub struct SharedBvhState {
    builder: BvhGpuBuilder,
    slot_a: OutputSlot,
    slot_b: OutputSlot,
    /// `0` → consumers read `slot_a`, build target is `slot_b`.
    current_slot: u8,
    pub(super) pending: Option<Pending>,
    /// Hash of the last successfully kicked scene state. Compared by
    /// the caller-supplied hash on the next kick.
    dirty_hash: Option<u64>,
    /// CPU mirror of the GPU BVH for CPU traversal consumers. `None`
    /// until the first full build resolves; thereafter refreshed on
    /// every successful build / refit swap.
    cpu_mirror: Option<CpuMirror>,
    /// Lifetime count of accepted [`Self::kick`] calls — every full
    /// build the orchestrator has committed to since construction.
    builds_kicked: u64,
    /// Lifetime count of accepted [`Self::kick_refit`] calls — every
    /// refit fast-path the orchestrator has committed to.
    refits_kicked: u64,
}

impl SharedBvhState {
    /// Build the GPU compute infrastructure + initial empty slots.
    /// `pipeline_cache` is forwarded to the LBVH builder — pass the
    /// engine's shared `wgpu::PipelineCache` to amortise compile time.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
        let builder = BvhGpuBuilder::new(device, queue, pipeline_cache);
        Self {
            builder,
            slot_a: OutputSlot::new(device, INITIAL_SLOT_CAPACITY),
            slot_b: OutputSlot::new(device, INITIAL_SLOT_CAPACITY),
            current_slot: 0,
            pending: None,
            dirty_hash: None,
            cpu_mirror: None,
            builds_kicked: 0,
            refits_kicked: 0,
        }
    }

    /// Lifetime count of accepted full builds. Equal to the number of
    /// times [`Self::kick`] (directly or via [`Self::kick_auto`])
    /// returned `Some`.
    pub fn builds_kicked(&self) -> u64 {
        self.builds_kicked
    }

    /// Lifetime count of accepted refit fast-paths. Equal to the
    /// number of times [`Self::kick_refit`] (directly or via
    /// [`Self::kick_auto`]) returned `Some`.
    pub fn refits_kicked(&self) -> u64 {
        self.refits_kicked
    }

    /// Borrow the CPU mirror of the currently-active BVH. `None` until
    /// the first full build resolves; thereafter refreshed on every
    /// successful build / refit swap. CPU consumers walk the tree with
    /// [`Bvh::for_each_aabb`] / [`Bvh::for_each_sphere`] / friends.
    pub fn current_cpu_bvh(&self) -> Option<&Bvh<u32>> {
        self.cpu_mirror.as_ref().map(|m| &m.bvh)
    }

    /// Borrow the per-leaf metadata mirror in **original input order**.
    /// CPU consumers index this by `bvh.leaves[k]` (= original position)
    /// to recover the [`LeafAabb`] of the leaf at sorted position `k`.
    /// Filter by `IS_COLLIDER` / `IS_VISIBLE_MESH` / etc. to scope the
    /// query to a consumer subset. `None` until the first full build
    /// resolves.
    pub fn current_cpu_leaf_aabbs(&self) -> Option<&[LeafAabb]> {
        self.cpu_mirror.as_ref().map(|m| m.leaf_aabbs.as_slice())
    }

    /// Borrow the BVH-nodes buffer for the currently-active slot.
    pub fn current_nodes(&self) -> &wgpu::Buffer {
        &self.slot(self.current_slot).nodes_buffer
    }

    /// Borrow the sorted-indices buffer for the currently-active slot.
    pub fn current_sorted_indices(&self) -> &wgpu::Buffer {
        &self.slot(self.current_slot).sorted_indices_buffer
    }

    /// Borrow the leaf-AABB buffer for the currently-active slot.
    pub fn current_leaf_aabbs(&self) -> &wgpu::Buffer {
        &self.slot(self.current_slot).leaf_aabbs_buffer
    }

    /// Number of valid primitives in the currently-active slot. `0`
    /// before any build has resolved.
    pub fn current_n(&self) -> u32 {
        self.slot(self.current_slot).n
    }

    /// Index (0 or 1) of the slot consumers are currently reading.
    /// Side-payload double-buffers should bind their own slot at this
    /// index.
    pub fn current_slot_index(&self) -> u8 {
        self.current_slot
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

    /// Start a new GPU build if `scene_hash` differs from the last
    /// successfully kicked hash. Returns `Some(BuildToken<'_>)` when a
    /// new build was committed and `None` when the kick was suppressed
    /// (the hash matched or a previous build is still in flight).
    ///
    /// The caller supplies `scene_hash` so each consumer can fold its
    /// own side-payload bytes (raymarch smoothness, collider mass,
    /// etc.) into the hash. A pure rebuild of `items + leaf_aabbs`
    /// would miss those changes and silently render stale frames.
    ///
    /// Side payloads are attached to the returned token via
    /// [`BuildToken::attach_payload`]; they fire once on swap and are
    /// dropped without running on build failure, keeping every
    /// consumer's parallel state in lockstep with the orchestrator.
    pub fn kick(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        items: Vec<(u32, Aabb)>,
        leaf_aabbs: Vec<LeafAabb>,
        scene_hash: u64,
    ) -> Option<BuildToken<'_>> {
        debug_assert_eq!(
            items.len(),
            leaf_aabbs.len(),
            "items and leaf_aabbs must align 1:1 — one entry per primitive",
        );
        if self.pending.is_some() {
            return None;
        }
        if Some(scene_hash) == self.dirty_hash {
            return None;
        }
        let n = items.len() as u32;
        let target_slot = self.current_slot ^ 1;

        let needed = (n as u64).max(1);
        self.slot_mut(target_slot).ensure_capacity(device, needed);

        let build = Bvh::<u32>::build_gpu(&mut self.builder, device, queue, items);
        self.pending = Some(Pending {
            op: PendingKind::Build(build),
            target_slot,
            leaf_aabbs,
            n,
            side_payloads: Vec::new(),
        });
        self.dirty_hash = Some(scene_hash);
        self.builds_kicked += 1;
        Some(BuildToken { shared: self })
    }

    /// Refit fast path: rewrite leaves with new AABBs and propagate
    /// internals over the existing topology, skipping morton + sort
    /// + Karras' internal-node pass. Returns `Some(BuildToken<'_>)`
    /// when a refit was committed, `None` when the kick was suppressed.
    ///
    /// Suppressed when:
    /// - A previous build / refit is still in flight.
    /// - `scene_hash` matches the last successfully kicked hash.
    /// - The current slot's `n` does not match `items.len()` (refit
    ///   requires the same cardinality + ordering as the previous
    ///   build; otherwise the caller should fall back to [`Self::kick`]).
    /// - There is no previous build in the builder's scratch (i.e.
    ///   `current_n() == 0`); a refit has nothing to start from.
    ///
    /// **Caller invariants** for a successful refit (silent corruption
    /// otherwise):
    /// - `items[i].0` is at the same array position as in the last
    ///   build. Only the AABBs are allowed to change.
    /// - The previous build's outputs still live in the builder's
    ///   scratch (no intermediate failed [`Self::kick`] has clobbered
    ///   them; failed kicks discard `pending` cleanly so this is true
    ///   in practice).
    pub fn kick_refit(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        items: Vec<(u32, Aabb)>,
        leaf_aabbs: Vec<LeafAabb>,
        scene_hash: u64,
    ) -> Option<BuildToken<'_>> {
        debug_assert_eq!(
            items.len(),
            leaf_aabbs.len(),
            "items and leaf_aabbs must align 1:1 — one entry per primitive",
        );
        if self.pending.is_some() {
            return None;
        }
        if Some(scene_hash) == self.dirty_hash {
            return None;
        }
        let n = items.len() as u32;
        if n == 0 || self.slot(self.current_slot).n != n {
            // Refit invariant: cardinality must match the previous
            // build. Caller must use kick() instead.
            return None;
        }
        let target_slot = self.current_slot ^ 1;
        let needed = n as u64;
        self.slot_mut(target_slot).ensure_capacity(device, needed);

        let refit = crate::gpu::refit_gpu::<u32>(&mut self.builder, device, queue, items);
        self.refits_kicked += 1;
        self.pending = Some(Pending {
            op: PendingKind::Refit(refit),
            target_slot,
            leaf_aabbs,
            n,
            side_payloads: Vec::new(),
        });
        self.dirty_hash = Some(scene_hash);
        Some(BuildToken { shared: self })
    }

    /// Drive the in-flight build forward. Must be called once per
    /// frame. Returns `Some(SwapInfo)` on the frame the swap actually
    /// happens; consumers maintaining parallel double-buffers should
    /// upload their data to `info.target_slot`.
    ///
    /// - `None` — no pending build, or pending build still in flight.
    /// - `Some(Ok(info))` — pending build resolved; result copied into
    ///   the target slot; every attached side-payload uploader fired
    ///   in registration order; CPU mirror refreshed; `current_slot`
    ///   flipped.
    /// - `Some(Err(_))` — build failed; pending dropped without
    ///   touching the slots; every attached side-payload closure is
    ///   dropped without running. Consumers keep using the previous
    ///   slot.
    pub fn poll_swap(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Option<Result<SwapInfo, BvhBuildError>> {
        device.poll(wgpu::PollType::Poll).ok()?;
        let pending = self.pending.as_mut()?;
        let outcome = pending.op.poll(device)?;
        let pending = self.pending.take().expect("just observed Some above");

        let outcome = match outcome {
            Ok(o) => o,
            Err(e) => {
                // Drop pending (and with it every captured side-payload
                // uploader). Consumers that attached payloads observe
                // the failure as "their captured data was never
                // uploaded"; their parallel buffers stay in their pre-
                // kick state.
                return Some(Err(e));
            }
        };

        let n = pending.n;
        let target_slot = pending.target_slot;
        if n > 0 {
            let total_nodes = (2 * n - 1) as u64;
            let nodes_bytes = total_nodes * std::mem::size_of::<BvhNode>() as u64;
            let indices_bytes = (n as u64) * 4;
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ome_bvh::shared::poll_swap_copy_encoder"),
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
        // Drain side-payload uploaders in registration order. Runs
        // BEFORE flipping `current_slot` so any consumer that re-reads
        // shared state inside its closure still sees the pre-swap
        // index — uploaders are passed `target_slot` explicitly.
        for upload in pending.side_payloads {
            upload(queue, target_slot);
        }
        // Refresh the CPU mirror — build replaces it, refit applies
        // in place over the existing topology. See [`CpuMirror`] for
        // the lifecycle contract.
        match outcome {
            PollOutcome::Build(result) => {
                self.cpu_mirror = Some(CpuMirror::from_build(result, pending.leaf_aabbs));
            }
            PollOutcome::Refit => {
                if let Some(mirror) = self.cpu_mirror.as_mut() {
                    mirror.apply_refit(pending.leaf_aabbs);
                }
            }
        }
        self.slot_mut(target_slot).n = n;
        self.current_slot = target_slot;
        Some(Ok(SwapInfo { target_slot, n }))
    }
}
