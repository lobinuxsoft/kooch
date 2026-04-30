//! Raymarch wrapper around the OmeAccel TLAS+BLAS pool.
//!
//! [`BvhState`] owns one `OmeAccel` (#360) and exposes a single-chunk
//! drive API consumed by `update_scene`. The pool's pre-allocated GPU
//! buffers replace the legacy global-BVH path that PR-2 retired —
//! bind-group references stay stable for the lifetime of the
//! renderer, so the renderer builds the scene bind group once at
//! construction.
//!
//! # Lifecycle per frame
//!
//! ```text
//!   1. update_scene collects every visible SDF primitive and folds
//!      per-role smoothness via `reduce_per_role_smoothness`.
//!   2. bvh_state.update_single_chunk(...):
//!        - hashes (leaf_aabbs ⊕ primitives); skips re-upload on hit
//!        - on miss: removes the prior chunk + reinserts the new one
//!        - always ticks `update_gpu` so `tlas_uniforms.k_*_global`
//!          tracks the per-frame reduce
//!   3. Renderer binds `bvh_state.buffers()` (group 1, bindings 5..=10)
//!      in the raymarch fragment pass.
//! ```
//!
//! # Empty scenes
//!
//! `update_single_chunk` evicts the chunk when `leaf_aabbs.is_empty()`
//! so the GPU sees `tlas_uniforms.num_chunks == 0`; the shader's
//! `eval_scene_bvh` short-circuit returns the union identity and the
//! fragment shader draws sky for every pixel.

mod state;

pub use state::BvhState;

#[cfg(test)]
mod tests;
