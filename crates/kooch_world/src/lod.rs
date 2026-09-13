//! Chunk-loading radii by LOD, one table every focus shares.

/// One LOD ring: chunks at this LOD level load if any active focus is
/// within `radius_meters` of them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LodRing {
    pub lod: u8,
    pub radius_meters: f32,
}

/// Engine-wide LOD ring table. The default, one ring × 256 m, suits editing; a planet-scale game
/// inserts its own before the streaming plugin's first tick.
#[derive(Debug, Clone)]
pub struct LodRingConfig {
    pub rings: Vec<LodRing>,
}

impl LodRingConfig {
    /// Number of LOD levels in the table.
    pub fn lod_count(&self) -> u8 {
        self.rings.len() as u8
    }
}

impl Default for LodRingConfig {
    fn default() -> Self {
        // Single LOD-0 ring × 256 m: ≈ 5³ = 125 cells per recompute,
        // < 1 ms wall-clock in the cache-gated activation. Editor-
        // comfortable. Gameplay configs are per-game opt-in.
        Self {
            rings: vec![LodRing {
                lod: 0,
                radius_meters: 256.0,
            }],
        }
    }
}

impl LodRingConfig {}
