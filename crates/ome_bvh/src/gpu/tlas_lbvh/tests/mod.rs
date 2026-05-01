//! GPU integration tests for the TLAS Karras LBVH pipeline.
//!
//! Every test acquires a wgpu device through
//! [`crate::gpu::builder::test_device::try_acquire`] and skips itself
//! gracefully when no adapter is available — CI without a display
//! falls into that path. The 16 hand-picked chunk centres in
//! [`helpers::TEST_CENTRES`] are shared across tests so any
//! divergence is attributable to the dispatch under test, not to
//! input drift.

mod aabb;
mod helpers;
mod internal;
mod leaves;
mod morton;
mod sort;
