//! Chunk-based world streaming: chunk identity and state, the focus component driving load and
//! unload, the LOD ring table and the [`ChunkManager`] between them. Hierarchical coordinates live
//! in `kooch_core::coord`.

pub mod activation;
pub mod chunk;
pub mod focus;
pub mod focus_cache;
pub mod lod;
pub mod manager;
pub mod plugin;
pub mod voxel;

pub use activation::{activate_chunks, activation_system};
pub use chunk::{BASE_CHUNK_SIZE_METERS, ChunkData, ChunkId, ChunkState, MAX_LOD_LEVEL};
// since PR #115 PR-1; consumers can keep importing it through `kooch_world`.
pub use focus::StreamingFocus;
pub use focus_cache::{DirtyFocusLod, FocusCacheState, FocusPosition};
pub use kooch_core::Aabb;
pub use lod::{LodRing, LodRingConfig};
pub use manager::ChunkManager;
pub use plugin::{
    DEFAULT_MAX_LOADS_PER_FRAME, DEFAULT_MAX_UNLOADS_PER_FRAME, WorldStreamingPlugin,
    world_streaming_system,
};
