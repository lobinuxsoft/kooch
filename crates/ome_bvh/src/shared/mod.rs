//! [`SharedBvhState`] — multi-consumer GPU BVH lifecycle.
//!
//! Owns a single [`crate::BvhGpuBuilder`] and a double-buffered pair
//! of [`OutputSlot`]s (`nodes + sorted_indices + leaf_aabbs`). Every
//! BVH consumer (raymarch, physics broadphase, frustum culling) binds
//! the same three buffers from the currently-active slot — that is
//! the "shared" of the name (#115 PR-5 AC 116).
//!
//! Side payloads (e.g. the raymarch's per-primitive smoothness) are
//! NOT owned by this struct. Consumers that need them maintain their
//! own parallel double-buffers and mirror the swap by listening to
//! [`SwapInfo::target_slot`] from
//! [`SharedBvhState::poll_swap`](state::SharedBvhState::poll_swap).
//!
//! # Hashing contract
//!
//! [`SharedBvhState::kick`](state::SharedBvhState::kick) takes the
//! scene hash from the caller rather than computing it. This lets
//! each consumer fold its side-payload bytes into the hash before
//! kicking — a smoothness-only change in raymarch must still trigger
//! a rebuild even though the items + leaves are byte-identical.
//!
//! # Module layout
//!
//! - [`slot`] — `OutputSlot` + per-buffer creation helpers.
//! - [`state`] — `SharedBvhState`, `BuildToken`, `Pending`,
//!   `PendingKind`, `SwapInfo`. The lifecycle (kick / kick_refit /
//!   poll_swap) and the type-state handle consumers receive on a
//!   committed kick.
//! - [`heuristic`] — `should_refit`, the cheap CPU heuristic that
//!   picks between rebuild and refit.

mod heuristic;
mod slot;
mod state;

pub use heuristic::should_refit;
pub use state::{BuildToken, SharedBvhState, SwapInfo};
