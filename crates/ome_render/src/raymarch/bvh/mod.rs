//! Raymarch wrapper around the engine-shared GPU BVH.
//!
//! The BVH lifecycle (build / swap / dirty hash) lives in
//! [`ome_bvh::SharedBvhState`]; this module adds the raymarch-only
//! [`RaymarchPayload[]`](crate::raymarch::instance::RaymarchPayload)
//! double-buffer that the fragment shader binds at slot 5.
//!
//! [`BvhState`] is the renderer-side handle: it owns a
//! `SharedBvhState`, mirrors its slot rotation onto the payload
//! buffer, and folds the raymarch-only smoothness bytes into the
//! caller-supplied scene hash. Every BVH-side concern (nodes,
//! sorted_indices, leaf_aabbs, capacity growth) is delegated.
//!
//! # Lifecycle per frame
//!
//! ```text
//!   1. (frame start) bvh_state.poll_swap(device, queue):
//!        - drives wgpu's map_async via SharedBvhState::poll_swap
//!        - if a swap occurred, the captured RaymarchPayload[] is
//!          uploaded onto the swapped-in slot.
//!   2. bvh_state.kick_if_dirty(...) — computes a hash that includes
//!      raymarch_payloads, forwards items + leaf_aabbs + hash to
//!      SharedBvhState::kick. No-op when the hash matches.
//!   3. Renderer binds bvh_state.current_*() buffers in the raymarch
//!      pass — guaranteed to be the last completed build's results.
//! ```
//!
//! # Empty / first-frame handling
//!
//! Before any build completes, `current_n() == 0` and the bind buffers
//! are minimal placeholders. The shader's `bvh_n == 0` branch renders
//! the sky, mirroring PR-3's `Bvh::empty()` semantics.

mod slots;
mod state;

pub use state::BvhState;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod gpu_tests;
