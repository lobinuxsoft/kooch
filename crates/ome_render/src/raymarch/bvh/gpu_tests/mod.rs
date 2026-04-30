//! GPU integration tests for [`super::BvhState`] (PR-4 of #115).
//!
//! Bypass the ECS layer: build a synthetic scene of N random spheres,
//! drive `BvhState::kick_if_dirty + poll_swap` to GPU completion, then
//! dispatch a small compute shader that mirrors `eval_scene_bvh` from
//! the production fragment shader.
//!
//! - [`byte_identical`] — two consecutive runs of the same scene must
//!   produce bit-identical float output (S9).
//! - [`lipschitz`] — BVH-driven path stays within `k_max + 4·ULP` of
//!   the brute-force baseline at points inside an inflated AABB (S10).
//! - [`bench`] — wall-clock perf at 1k / 10k / 65k primitives,
//!   `#[ignore]`-gated (S11).
//!
//! Shared helpers live in [`harness`]; the WGSL compute kernel that
//! both tests dispatch is in [`shader`]. The kernel is intentionally
//! a private string here rather than a shared file — exposing the
//! production raymarch internals just to dedupe ~80 lines of WGSL is
//! a worse trade.

mod bench;
mod byte_identical;
mod harness;
mod lipschitz;
mod multirole;
mod shader;
