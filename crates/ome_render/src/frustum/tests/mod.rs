//! GPU correctness tests for [`super::cull::FrustumCull`], split by
//! concern so no single file crosses the no-monolithic threshold:
//!
//! - [`harness`] — shared GPU device acquisition, scene fixtures,
//!   plane-AABB CPU reference, dispatch + readback. `pub(super)` so
//!   the sibling `bench` module imports the same fixtures.
//! - [`cull`] — frustum cull correctness (all-inside, all-outside,
//!   10k cubes byte-identical vs CPU brute force, IS_VISIBLE_MESH gate).
//! - [`integration`] — AC 116 multi-consumer integration (raymarch
//!   buffers + physics broadphase + frustum cull all reading from a
//!   single `SharedBvhState`).
//!
//! The `pub(super) use harness::*` re-export keeps `bench.rs`'s
//! existing `use super::tests::{...}` paths working unchanged.

mod cull;
mod harness;
mod integration;

pub(super) use harness::{
    axis_aligned_box_frustum, dispatch_and_readback, drive_build_to_completion,
    try_acquire_device, visible_mesh_scene,
};
