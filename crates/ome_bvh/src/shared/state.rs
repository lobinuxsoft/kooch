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
//! in lockstep with the orchestrator's `pending` by construction.
//!
//! The token is opaque: it cannot be constructed by the consumer, only
//! by the orchestrator. While a token lives the orchestrator's
//! `pending` is guaranteed populated, so [`BuildToken::attach_payload`]
//! never has to defend against a torn invariant.

use crate::leaf::LeafAabb;
use crate::{Aabb, Bvh, BvhBuildError, BvhGpuBuild, BvhGpuBuilder, BvhGpuRefit, BvhNode};

use super::slot::{INITIAL_SLOT_CAPACITY, OutputSlot};

/// Side-payload uploader stored in [`Pending::side_payloads`]. Runs at
/// swap time with `(queue, target_slot)`; dropped without running on
/// build failure.
///
/// `'static` because the kick → swap span crosses frame boundaries —
/// any captured wgpu handles must be owned (e.g. cloned `wgpu::Buffer`,
/// which is `Arc` internally) and any captured payload data must be
/// moved. `Send + Sync` so [`SharedBvhState`] stays a valid ECS
/// resource on engines that drive `poll_swap` from a worker thread.
type SidePayloadUploader = Box<dyn FnOnce(&wgpu::Queue, u8) + Send + Sync>;

/// Discriminator between an in-flight full build and an in-flight
/// refit. Both operate on the builder's scratch buffers and resolve
/// at the same point in the lifecycle ([`SharedBvhState::poll_swap`]),
/// so the orchestrator carries them through one common state slot.
enum PendingKind {
    Build(BvhGpuBuild<u32>),
    Refit(BvhGpuRefit),
}

impl PendingKind {
    fn poll(&mut self, device: &wgpu::Device) -> Option<Result<(), BvhBuildError>> {
        match self {
            // BvhGpuBuild::poll returns the full Bvh<T>; we drop it
            // (the orchestrator path stays GPU-resident).
            Self::Build(op) => op.poll(device).map(|r| r.map(|_| ())),
            Self::Refit(op) => op.poll(device),
        }
    }
}

/// In-flight build or refit awaiting GPU completion.
struct Pending {
    op: PendingKind,
    /// Slot index (0 or 1) the result will land in.
    target_slot: u8,
    /// Per-leaf metadata captured at kick time. Uploaded to the target
    /// slot's `leaf_aabbs_buffer` at swap. Held on the CPU because the
    /// LBVH builder doesn't see this metadata.
    leaf_aabbs: Vec<LeafAabb>,
    /// Number of leaves submitted to the build.
    n: u32,
    /// Side-payload uploaders attached by consumers via
    /// [`BuildToken::attach_payload`]. Drained on swap success in the
    /// order they were attached; dropped without running on swap
    /// failure (the captured data dies with them).
    side_payloads: Vec<SidePayloadUploader>,
}

/// Information published when [`SharedBvhState::poll_swap`] resolves a
/// pending build. Side-payload consumers mirror their double-buffer
/// swap onto [`Self::target_slot`] and copy `n` items into it.
#[derive(Clone, Copy, Debug)]
pub struct SwapInfo {
    /// The slot index (0 or 1) that just became `current`. Side-
    /// payload consumers should upload their parallel data to this
    /// slot.
    pub target_slot: u8,
    /// Number of leaves in the resolved build.
    pub n: u32,
}

/// Opaque handle to an *accepted* kick. Returned by
/// [`SharedBvhState::kick`] / [`SharedBvhState::kick_refit`] only when
/// the kick was committed; suppressed kicks return `None`.
///
/// The token's existence is the type-level guarantee that
/// [`SharedBvhState::pending`] is `Some` and that the slot / cardinality
/// returned by [`Self::target_slot`] / [`Self::n`] are stable until the
/// caller drops the token. While the token lives the orchestrator is
/// borrowed mutably — drop it before calling any other method on the
/// shared state.
///
/// Side-payload consumers (raymarch payload buffer, future per-collider
/// metadata, ...) attach their upload via [`Self::attach_payload`]. On
/// swap success every attached uploader runs in registration order; on
/// swap failure they are dropped without running, so any captured data
/// dies cleanly.
pub struct BuildToken<'a> {
    shared: &'a mut SharedBvhState,
}

impl BuildToken<'_> {
    /// Slot index (0 or 1) the resolved build will land in. Mirrors
    /// the `target_slot` later published in the matching [`SwapInfo`];
    /// consumers can pre-grow their parallel buffers against this
    /// index without waiting for [`SharedBvhState::poll_swap`].
    pub fn target_slot(&self) -> u8 {
        self.pending().target_slot
    }

    /// Number of leaves in the kicked build. Same as the `n` published
    /// in the matching [`SwapInfo`].
    pub fn n(&self) -> u32 {
        self.pending().n
    }

    /// Register a side-payload uploader for this build. The closure
    /// runs once on the frame [`SharedBvhState::poll_swap`] reports a
    /// successful swap, with `(queue, target_slot)` — the consumer
    /// uses `target_slot` to pick which of its parallel double-buffer
    /// slots to write to.
    ///
    /// Multiple calls stack: each consumer with its own side payload
    /// can attach independently; uploaders run in registration order
    /// on swap. On build failure every attached closure is dropped
    /// without running, so the captured payload data is released and
    /// the consumer's parallel buffers stay in their pre-kick state.
    pub fn attach_payload<F>(&mut self, uploader: F)
    where
        F: FnOnce(&wgpu::Queue, u8) + Send + Sync + 'static,
    {
        self.pending_mut().side_payloads.push(Box::new(uploader));
    }

    fn pending(&self) -> &Pending {
        self.shared
            .pending
            .as_ref()
            .expect("BuildToken always references a live Pending")
    }

    fn pending_mut(&mut self) -> &mut Pending {
        self.shared
            .pending
            .as_mut()
            .expect("BuildToken always references a live Pending")
    }
}

/// Multi-consumer double-buffered GPU BVH state. Held as a single
/// resource shared by every BVH consumer in the engine.
pub struct SharedBvhState {
    builder: BvhGpuBuilder,
    slot_a: OutputSlot,
    slot_b: OutputSlot,
    /// `0` → consumers read `slot_a`, build target is `slot_b`.
    current_slot: u8,
    pending: Option<Pending>,
    /// Hash of the last successfully kicked scene state. Compared by
    /// the caller-supplied hash on the next kick.
    dirty_hash: Option<u64>,
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
        }
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
    ///   in registration order; `current_slot` flipped.
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

        if let Err(e) = outcome {
            // Drop pending (and with it every captured side-payload
            // uploader). Consumers that attached payloads observe the
            // failure as "their captured data was never uploaded";
            // their parallel buffers stay in their pre-kick state.
            return Some(Err(e));
        }

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
        self.slot_mut(target_slot).n = n;
        self.current_slot = target_slot;
        Some(Ok(SwapInfo { target_slot, n }))
    }
}
