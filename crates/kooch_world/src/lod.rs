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

    /// Radius for the given LOD level, or `0.0` when out of range.
    pub fn radius_for(&self, lod: u8) -> f32 {
        self.rings
            .iter()
            .find(|r| r.lod == lod)
            .map(|r| r.radius_meters)
            .unwrap_or(0.0)
    }

    /// Largest radius in the table — the outer streaming horizon.
    pub fn max_radius(&self) -> f32 {
        self.rings
            .iter()
            .map(|r| r.radius_meters)
            .fold(0.0_f32, f32::max)
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

impl LodRingConfig {
    /// Aspirational planet-scale config: 4 rings at 512 m / 2 km /
    /// 8 km / 32 km. **Will hitch with the current PR #318 cache-only
    /// activation.** Use only after the #327 streaming performance
    /// roadmap lands PHASE 1 (#319 + #322).
    ///
    /// Kept as a named factory so games that want to opt in have a
    /// single call site to update once PHASE 1 ships.
    pub fn aspirational_planet_scale() -> Self {
        Self {
            rings: vec![
                LodRing {
                    lod: 0,
                    radius_meters: 512.0,
                },
                LodRing {
                    lod: 1,
                    radius_meters: 2000.0,
                },
                LodRing {
                    lod: 2,
                    radius_meters: 8000.0,
                },
                LodRing {
                    lod: 3,
                    radius_meters: 32000.0,
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_conservative_single_ring() {
        // Editor-default: 1 LOD-0 ring × 256 m.
        let cfg = LodRingConfig::default();
        assert_eq!(cfg.lod_count(), 1);
        assert_eq!(cfg.radius_for(0), 256.0);
    }

    #[test]
    fn aspirational_config_has_four_rings() {
        let cfg = LodRingConfig::aspirational_planet_scale();
        assert_eq!(cfg.lod_count(), 4);
        assert_eq!(cfg.radius_for(0), 512.0);
        assert_eq!(cfg.radius_for(1), 2000.0);
        assert_eq!(cfg.radius_for(2), 8000.0);
        assert_eq!(cfg.radius_for(3), 32000.0);
    }

    #[test]
    fn radius_for_out_of_range_is_zero() {
        let cfg = LodRingConfig::default();
        assert_eq!(cfg.radius_for(99), 0.0);
    }

    #[test]
    fn max_radius_returns_largest() {
        let cfg = LodRingConfig::aspirational_planet_scale();
        assert_eq!(cfg.max_radius(), 32000.0);
    }

    #[test]
    fn max_radius_empty_config_is_zero() {
        let cfg = LodRingConfig { rings: vec![] };
        assert_eq!(cfg.max_radius(), 0.0);
    }

    #[test]
    fn aspirational_radii_strictly_increase() {
        let cfg = LodRingConfig::aspirational_planet_scale();
        let mut prev = 0.0;
        for r in &cfg.rings {
            assert!(r.radius_meters > prev, "ring {:?} not strictly larger", r);
            prev = r.radius_meters;
        }
    }
}
