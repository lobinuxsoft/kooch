//! Per-build knobs for the multi-LOD chain construction.

/// Configuration for [`super::build_meshlets_lod_chain`].
#[derive(Debug, Clone, Copy)]
pub struct LodConfig {
    /// Maximum number of LOD levels to attempt past LOD 0. The chain
    /// stops early when `meshopt::simplify` cannot reduce the index
    /// count further (typically when the topology is too constrained
    /// to simplify any more). Default: 6.
    pub max_levels: usize,
    /// Initial simplify error tolerance in mesh units. Doubles each
    /// level; balanced default: 0.01.
    pub initial_error: f32,
    /// Target ratio for index reduction per level. 0.5 halves the
    /// triangle count each step; 0.7 is gentler.
    pub target_ratio: f32,
}

impl Default for LodConfig {
    fn default() -> Self {
        Self {
            max_levels: 6,
            initial_error: 0.01,
            target_ratio: 0.5,
        }
    }
}
