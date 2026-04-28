//! Double-buffered GPU BVH state for the raymarch primitive culling
//! integration (PR-4 of #115).
//!
//! [`BvhState`] owns a [`ome_bvh::BvhGpuBuilder`] (the reusable LBVH
//! compute pipeline + scratch buffers from PR-3) plus two output slots
//! — `slot_a` and `slot_b` — each holding a stable copy of the last
//! completed build's outputs (nodes, sorted indices, leaf AABBs). The
//! renderer always reads from `current_slot`; pending builds copy their
//! results into the OTHER slot at swap time, so the renderer never
//! observes a half-written buffer and the GPU avoids a stall on the
//! read-after-write hazard a single shared buffer would introduce.
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
//!
//! # Module layout
//!
//! - [`slot`] — `OutputSlot`: per-slot buffer set + capacity grow.
//! - [`state`] — `BvhState`: lifecycle (kick/poll_swap), hash, accessors.
//! - [`tests`] — CPU-only unit tests for the hash function.
//! - [`gpu_tests`] — GPU integration tests (byte-identical determinism,
//!   Lipschitz-bounded vs fullscan, perf scaling bench).

mod slot;
mod state;

pub use state::BvhState;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod gpu_tests;
