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

/// Engine-wide LOD ring table. Default: 4 rings matching the
/// example in issue #54 body — 512 m / 2 km / 8 km / 32 km radii for
/// LOD 0 / 1 / 2 / 3.
///
/// The table is intentionally tunable per-game; defaults are picked
/// for an Earth-scale planet with sub-meter detail near the camera.
/// A space-game with sparser scale will want a different curve.
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
    fn default_has_four_rings() {
        let cfg = LodRingConfig::default();
        assert_eq!(cfg.lod_count(), 4);
    }

    #[test]
    fn default_radii_match_issue_54_body() {
        let cfg = LodRingConfig::default();
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
        let cfg = LodRingConfig::default();
        assert_eq!(cfg.max_radius(), 32000.0);
    }

    #[test]
    fn max_radius_empty_config_is_zero() {
        let cfg = LodRingConfig { rings: vec![] };
        assert_eq!(cfg.max_radius(), 0.0);
    }

    #[test]
    fn radii_strictly_increase_in_default() {
        let cfg = LodRingConfig::default();
        let mut prev = 0.0;
        for r in &cfg.rings {
            assert!(r.radius_meters > prev, "ring {:?} not strictly larger", r);
            prev = r.radius_meters;
        }
    }
}
