//! ome_physics — physics for oh_my_engine.
//!
//! Currently ships the CPU broadphase consumer of the engine-shared
//! BVH (`#42`, S4 of `#115` PR-5). Narrowphase, contact resolution,
//! and SDF collision response remain ahead.

pub mod broadphase;

pub use broadphase::{BroadphasePairs, CollisionPair};

/// Placeholder for future runtime initialization. Kept while the
/// engine plugin wiring still references it; will be removed when
/// physics gets a proper plugin entry point.
pub fn init() {
    tracing::info!("ome_physics initialized");
}
