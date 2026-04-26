//! ome_world — chunk-based world streaming for `oh_my_engine`.
//!
//! Defines the chunk identity / state types, the streaming-focus
//! component that drives load/unload decisions, the LOD ring
//! configuration, and the [`ChunkManager`] resource that mediates
//! between them.
//!
//! Hierarchical coordinates live in `ome_core::coord` (issue #50).
//! Sparse SDF storage (#136), BVH (#115), Edit Baker (#309), physics
//! regions (#311) and persistent edit logs (#312) compose with this
//! crate as separate concerns; see issue #54 + epic #313 for the
//! roadmap.

pub mod chunk;
pub mod focus;
pub mod lod;

pub use chunk::{
    Aabb, BASE_CHUNK_SIZE_METERS, ChunkData, ChunkId, ChunkState, MAX_LOD_LEVEL,
};
pub use focus::StreamingFocus;
pub use lod::{LodRing, LodRingConfig};
