//! Per-pass dispatch helpers for the TLAS Karras LBVH GPU pipeline.
//!
//! Each function records its compute pass into the caller-supplied
//! encoder; the orchestration loop in
//! [`super::TlasGpuBuilder::dispatch_rebuild`] (lands in commit 7)
//! chains them together so morton + sort + leaves + internal + aabb
//! share a single submission and a single CPU side-effect window.
//!
//! One submodule per pass — every dispatch is its own `impl` block on
//! [`super::TlasGpuBuilder`]. They share nothing but the builder type
//! and a couple of imports, so the split is a clean per-pass boundary.

mod aabb;
mod internal;
mod leaves;
mod morton;
mod sort;
