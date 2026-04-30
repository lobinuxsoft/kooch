//! Configuration caps and error types for the TLAS+BLAS pool
//! acceleration structure (issue #360).
//!
//! Caps are pre-allocated at `OmeAccel::new` — exhaustion at hot-path
//! insert time surfaces as a typed error instead of a panic, so the
//! streaming layer can choose to evict + retry rather than abort the
//! frame.

/// Per-pool capacity caps. `Default` provides the profiling-driven
/// values from the issue body. Override at `OmeAccel::new` time when
/// the streaming budget calls for it (e.g. larger `max_chunks` for
/// dense urban scenes, larger `max_primitives` for high-detail
/// builds).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AccelCaps {
    pub max_chunks: u32,
    pub max_nodes: u32,
    pub max_leaves: u32,
    pub max_primitives: u32,
}

impl Default for AccelCaps {
    fn default() -> Self {
        Self {
            max_chunks: 1024,
            max_nodes: 2_097_152,
            max_leaves: 1_048_576,
            max_primitives: 1_048_576,
        }
    }
}

impl AccelCaps {
    /// Tiny cap set for unit / integration tests where allocating the
    /// full default ~96 MB of GPU buffers is wasteful.
    pub const TEST: Self = Self {
        max_chunks: 16,
        max_nodes: 16_384,
        max_leaves: 8_192,
        max_primitives: 8_192,
    };
}

/// Errors surfaced by `OmeAccel` operations. None of these abort the
/// frame — the streaming layer is expected to convert them into
/// eviction + retry logic.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AccelError {
    /// `chunk_descriptors` pool exhausted. Free a chunk slot or
    /// raise `AccelCaps::max_chunks`.
    OutOfChunkSlots,
    /// `bvh_nodes_pool` exhausted. Coalesce free ranges or raise
    /// `AccelCaps::max_nodes`.
    OutOfNodes,
    /// `leaf_aabbs_pool` exhausted. Coalesce free ranges or raise
    /// `AccelCaps::max_leaves`.
    OutOfLeaves,
    /// `primitives_pool` exhausted. Coalesce free ranges or raise
    /// `AccelCaps::max_primitives`.
    OutOfPrimitives,
    /// A chunk coordinate referenced by `remove_chunk` / `refit_chunk`
    /// is not resident.
    UnknownChunk,
    /// `insert_chunk` was called with zero primitives. The pool
    /// contract requires at least one leaf per BLAS so `node_count`
    /// = `2 * leaf_count - 1` stays well-defined.
    EmptyPrimitives,
}

impl core::fmt::Display for AccelError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::OutOfChunkSlots => write!(f, "TLAS chunk_descriptors pool exhausted"),
            Self::OutOfNodes => write!(f, "BLAS bvh_nodes_pool exhausted"),
            Self::OutOfLeaves => write!(f, "BLAS leaf_aabbs_pool exhausted"),
            Self::OutOfPrimitives => write!(f, "BLAS primitives_pool exhausted"),
            Self::UnknownChunk => write!(f, "chunk coordinate not resident"),
            Self::EmptyPrimitives => write!(f, "insert_chunk requires non-empty primitives"),
        }
    }
}

impl std::error::Error for AccelError {}
