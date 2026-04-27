//! ome_bvh — spatial acceleration structure shared across the engine.
//!
//! Provides a generic LBVH (Linear BVH) usable by world streaming
//! (chunk activation), physics broadphase, raymarch primitive culling,
//! and frustum / light culling. The CPU builder (this crate) and the
//! eventual GPU compute builder (issue #115 PR-3) emit byte-identical
//! [`BvhNode`] arrays — consumers see the same query API regardless of
//! where the build happened.
//!
//! # Roadmap
//!
//! This crate is **PR-1 of 5** for issue #115:
//!
//! 1. **PR-1 (this PR)**: foundation — `Aabb`, `MortonCode`, `BvhNode`,
//!    CPU LBVH builder, generic `Bvh<T: Copy>`, query API
//!    (sphere / AABB / ray / point).
//! 2. **PR-2**: integration with `ome_world::activation` — replaces the
//!    brute-force `chunks_within_sphere`, re-registers the streaming
//!    system on the schedule (closes the deferral from PR #315 / #54).
//! 3. **PR-3**: GPU compute build — Morton + radix sort + LBVH builder
//!    in WGSL compute shaders. CPU build stays as fallback / debug.
//! 4. **PR-4**: WGSL query library + raymarch (#22) integration.
//! 5. **PR-5**: collision broadphase (#40) + frustum / light culling.

pub mod aabb;
pub mod bvh;
pub mod morton;
pub mod node;
pub mod query;

pub use aabb::Aabb;
pub use bvh::Bvh;
pub use morton::MortonCode;
pub use node::BvhNode;
pub use query::MAX_STACK_DEPTH;
