//! Lifecycle handles for an in-flight kick: the [`Pending`] state and
//! the consumer-facing [`BuildToken`] / [`SwapInfo`] published by
//! [`SharedBvhState`](super::state::SharedBvhState).
//!
//! The orchestrator's invariant — "while a `Pending` is set there is
//! exactly one `BuildToken` outstanding, and every side-payload upload
//! the consumers care about is captured inside `Pending::side_payloads`"
//! — is enforced by visibility: only this module mints a [`BuildToken`]
//! and only `state.rs` mutates the surrounding [`SharedBvhState::pending`]
//! field.

use crate::leaf::LeafAabb;
use crate::{BvhBuildError, BvhGpuBuild, BvhGpuBuildResult, BvhGpuRefit};

use super::state::SharedBvhState;

/// Side-payload uploader stored in [`Pending::side_payloads`]. Runs at
/// swap time with `(queue, target_slot)`; dropped without running on
/// build failure.
///
/// `'static` because the kick → swap span crosses frame boundaries —
/// any captured wgpu handles must be owned (e.g. cloned `wgpu::Buffer`,
/// which is `Arc` internally) and any captured payload data must be
/// moved. `Send + Sync` so [`SharedBvhState`] stays a valid ECS
/// resource on engines that drive `poll_swap` from a worker thread.
pub(super) type SidePayloadUploader = Box<dyn FnOnce(&wgpu::Queue, u8) + Send + Sync>;

/// Outcome of a resolved [`PendingKind::poll`] handed back to
/// [`SharedBvhState::poll_swap`] so it can refresh the CPU mirror.
pub(super) enum PollOutcome {
    /// Full build resolved. Carries the readback the GPU build path
    /// already paid for: a CPU `Bvh<u32>` plus the `sorted_indices`
    /// permutation the refit fast path needs.
    Build(BvhGpuBuildResult<u32>),
    /// Refit resolved. Topology unchanged; the orchestrator refits
    /// the existing CPU mirror in place.
    Refit,
}

/// Discriminator between an in-flight full build and an in-flight
/// refit. Both operate on the builder's scratch buffers and resolve
/// at the same point in the lifecycle ([`SharedBvhState::poll_swap`]),
/// so the orchestrator carries them through one common state slot.
pub(super) enum PendingKind {
    Build(BvhGpuBuild<u32>),
    Refit(BvhGpuRefit),
}

impl PendingKind {
    pub(super) fn poll(
        &mut self,
        device: &wgpu::Device,
    ) -> Option<Result<PollOutcome, BvhBuildError>> {
        match self {
            Self::Build(op) => op.poll(device).map(|r| r.map(PollOutcome::Build)),
            Self::Refit(op) => op.poll(device).map(|r| r.map(|()| PollOutcome::Refit)),
        }
    }
}

/// In-flight build or refit awaiting GPU completion.
pub(super) struct Pending {
    pub(super) op: PendingKind,
    /// Slot index (0 or 1) the result will land in.
    pub(super) target_slot: u8,
    /// Per-leaf metadata captured at kick time. Uploaded to the target
    /// slot's `leaf_aabbs_buffer` at swap. Held on the CPU because the
    /// LBVH builder doesn't see this metadata.
    pub(super) leaf_aabbs: Vec<LeafAabb>,
    /// Number of leaves submitted to the build.
    pub(super) n: u32,
    /// Side-payload uploaders attached by consumers via
    /// [`BuildToken::attach_payload`]. Drained on swap success in the
    /// order they were attached; dropped without running on swap
    /// failure (the captured data dies with them).
    pub(super) side_payloads: Vec<SidePayloadUploader>,
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
/// [`SharedBvhState`]'s pending state is `Some` and that the slot /
/// cardinality returned by [`Self::target_slot`] / [`Self::n`] are
/// stable until the caller drops the token. While the token lives the
/// orchestrator is borrowed mutably — drop it before calling any other
/// method on the shared state.
///
/// Side-payload consumers (raymarch payload buffer, future per-collider
/// metadata, ...) attach their upload via [`Self::attach_payload`]. On
/// swap success every attached uploader runs in registration order; on
/// swap failure they are dropped without running, so any captured data
/// dies cleanly.
pub struct BuildToken<'a> {
    pub(super) shared: &'a mut SharedBvhState,
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
