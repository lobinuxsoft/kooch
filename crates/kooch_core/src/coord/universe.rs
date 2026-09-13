//! Top-level absolute position in the universe.

use glam::{DVec3, IVec3};

/// Side length of a universe sector, in meters.
pub const SECTOR_SIZE_METERS: f64 = 1024.0;

/// Half a sector — the half-open boundary at which `offset` wraps and
/// `sector` increments. `offset.{x,y,z} ∈ [-SECTOR_HALF, SECTOR_HALF)`.
pub const SECTOR_HALF: f64 = SECTOR_SIZE_METERS * 0.5;

/// Absolute world position, split into sector index + within-sector offset. The canonical form
/// keeps `offset` normalised to `[-SECTOR_HALF, SECTOR_HALF)` on every axis; arithmetic helpers
/// re-normalise automatically.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UniverseCoord {
    pub sector: IVec3,
    pub offset: DVec3,
}

impl UniverseCoord {
    /// Origin of the universe.
    pub const ZERO: Self = Self {
        sector: IVec3::ZERO,
        offset: DVec3::ZERO,
    };

    /// Direct constructor. Does **not** normalise — pass an offset outside `[-SECTOR_HALF,
    /// SECTOR_HALF)` and you'll get a non-canonical representation.
    pub const fn new(sector: IVec3, offset: DVec3) -> Self {
        Self { sector, offset }
    }

    /// Build a canonical `UniverseCoord` from an absolute f64 world
    /// position. The position is split into sector index + within-sector
    /// offset such that the offset lands in `[-SECTOR_HALF, SECTOR_HALF)`.
    pub fn from_dvec3(world: DVec3) -> Self {
        // Add SECTOR_HALF then floor / SECTOR_SIZE: this picks the sector index whose [-half, half)
        // range contains `world`. Plain `round` would put the boundary at the half integer and
        // behave inconsistently on negative values where `round` ties away from zero.
        let shifted = world + DVec3::splat(SECTOR_HALF);
        let sector_f = (shifted / SECTOR_SIZE_METERS).floor();
        let sector = IVec3::new(sector_f.x as i32, sector_f.y as i32, sector_f.z as i32);
        let offset = world - sector_f * SECTOR_SIZE_METERS;
        Self { sector, offset }
    }

    /// Convert back to an absolute f64 world position.
    pub fn to_dvec3(&self) -> DVec3 {
        DVec3::new(
            self.sector.x as f64,
            self.sector.y as f64,
            self.sector.z as f64,
        ) * SECTOR_SIZE_METERS
            + self.offset
    }

    /// Re-canonicalise so that `offset.{x,y,z} ∈ [-SECTOR_HALF, SECTOR_HALF)`.
    /// Useful after manual construction or chained additions.
    pub fn normalised(self) -> Self {
        Self::from_dvec3(self.to_dvec3())
    }

    /// Add a relative displacement (in meters) and return the canonical
    /// result. The sector index will increment / decrement automatically
    /// when the offset crosses a sector boundary.
    pub fn translated(self, delta: DVec3) -> Self {
        Self::from_dvec3(self.to_dvec3() + delta)
    }

    /// Vector from `self` to `other` in meters. Returns [`DVec3`] to
    /// preserve full precision; cast to `Vec3` only when the result is
    /// guaranteed small (e.g. within camera range).
    pub fn delta_to(&self, other: &Self) -> DVec3 {
        other.to_dvec3() - self.to_dvec3()
    }
}

impl Default for UniverseCoord {
    fn default() -> Self {
        Self::ZERO
    }
}

#[cfg(test)]
mod tests;
