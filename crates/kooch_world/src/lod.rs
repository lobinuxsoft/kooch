//! LOD ring configuration — engine-wide chunk-loading radii by LOD level.
//!
//! Centralises the answer to "for chunks at LOD `N`, how close to a
//! focus must they be to load?" so all focuses share one ground truth.
//! Per-focus overrides can layer on top later (separate issue) when
//! gameplay demands it; for the warmup the global table is enough.

/// One LOD ring: chunks at this LOD level load if any active focus is
/// within `radius_meters` of them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LodRing {
    pub lod: u8,
    pub radius_meters: f32,
}

/// Engine-wide LOD ring table.
///
/// **Default = single ring × 256 m** — conservative for editor /
/// scene-authoring work. Picked so the current cache-gated activation
/// (PR #318) doesn't spike on first-seen or boundary cross. Each
/// recompute touches at most ~5³ = 125 cells.
///
/// **Per-game gameplay config is opt-in**: a planet-scale game wants
/// 4 rings at 512 m / 2 km / 8 km / 32 km (the original aspirational
/// default), but those produce frame spikes until the streaming
/// performance roadmap lands (#327 epic — incremental delta in #319,
/// async loading in #322, GPU-driven in #325). When PHASE 1 of #327
/// merges, the default can grow back to multi-LOD without hitches.
///
/// Override per-game by inserting a custom `LodRingConfig` resource
/// before the streaming plugin's first tick.
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

    // Gameplay-grade aspirational config (kept here as a reference
    // for when the streaming-performance roadmap (#327) catches up).
    // Restore as the default after #319 incremental delta + #322
    // async loading land.
}

impl LodRingConfig {}
