//! Top-level absolute position in the universe.
//!
//! [`UniverseCoord`] combines an integer sector index with a high-precision
//! offset within the sector. This split lets the engine address positions
//! anywhere on a planet — and beyond — without f32 precision loss far
//! from the origin.

use glam::{DVec3, IVec3};

/// Side length of a universe sector, in meters.
///
/// Picked at 1024 m so that:
/// - The within-sector offset stays well inside f64 precision over a
///   single planet (R ≈ 6000 km → at most ~6000 sectors crossed; offset
///   itself is bounded to ±512 m).
/// - Casting the offset to f32 for camera-relative GPU use is lossless
///   for any value within the sector.
/// - i32 sector indices cover roughly 2.1 × 10¹² m per axis, more than
///   the observable universe.
pub const SECTOR_SIZE_METERS: f64 = 1024.0;

/// Half a sector — the half-open boundary at which `offset` wraps and
/// `sector` increments. `offset.{x,y,z} ∈ [-SECTOR_HALF, SECTOR_HALF)`.
pub const SECTOR_HALF: f64 = SECTOR_SIZE_METERS * 0.5;

/// Absolute world position, split into sector index + within-sector
/// offset. The canonical form keeps `offset` normalised to
/// `[-SECTOR_HALF, SECTOR_HALF)` on every axis; arithmetic helpers
/// re-normalise automatically.
///
/// # Coordinate convention
///
/// Sector `(0, 0, 0)` covers world space `[-SECTOR_HALF, SECTOR_HALF)`
/// on each axis. Sector `(1, 0, 0)` covers `[SECTOR_HALF, 3·SECTOR_HALF)`
/// in `x`, etc. The lower edge of each sector is inclusive, the upper
/// edge exclusive — there is exactly one sector that owns any given
/// world point.
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

    /// Direct constructor. Does **not** normalise — pass an offset
    /// outside `[-SECTOR_HALF, SECTOR_HALF)` and you'll get a non-canonical
    /// representation. Use [`Self::from_dvec3`] when you start from an
    /// arbitrary world position, or [`Self::normalised`] to canonicalise
    /// after manual construction.
    pub const fn new(sector: IVec3, offset: DVec3) -> Self {
        Self { sector, offset }
    }

    /// Build a canonical `UniverseCoord` from an absolute f64 world
    /// position. The position is split into sector index + within-sector
    /// offset such that the offset lands in `[-SECTOR_HALF, SECTOR_HALF)`.
    pub fn from_dvec3(world: DVec3) -> Self {
        // Add SECTOR_HALF then floor / SECTOR_SIZE: this picks the
        // sector index whose [-half, half) range contains `world`.
        // Plain `round` would put the boundary at the half integer and
        // behave inconsistently on negative values where `round` ties
        // away from zero.
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
mod tests {
    use super::*;

    const EPS: f64 = 1e-9;

    #[test]
    fn zero_round_trip() {
        let z = UniverseCoord::ZERO;
        assert_eq!(z.to_dvec3(), DVec3::ZERO);
        assert_eq!(UniverseCoord::from_dvec3(DVec3::ZERO), UniverseCoord::ZERO);
    }

    #[test]
    fn round_trip_arbitrary_position() {
        let world = DVec3::new(1234.5, -678.9, 0.001);
        let uc = UniverseCoord::from_dvec3(world);
        assert!((uc.to_dvec3() - world).length() < EPS);
    }

    #[test]
    fn sector_zero_covers_centred_range() {
        // Center of sector 0.
        assert_eq!(
            UniverseCoord::from_dvec3(DVec3::ZERO).sector,
            IVec3::ZERO
        );
        // Just inside the upper edge — still sector 0 (upper exclusive).
        let near_edge = UniverseCoord::from_dvec3(DVec3::new(SECTOR_HALF - 1.0, 0.0, 0.0));
        assert_eq!(near_edge.sector, IVec3::ZERO);
        // At the lower edge — sector 0 (lower inclusive).
        let lower_edge = UniverseCoord::from_dvec3(DVec3::new(-SECTOR_HALF, 0.0, 0.0));
        assert_eq!(lower_edge.sector, IVec3::ZERO);
    }

    #[test]
    fn crossing_upper_boundary_increments_sector() {
        // SECTOR_HALF = 512. (513, 0, 0) lands in sector (1, 0, 0)
        // because the upper edge is exclusive.
        let uc = UniverseCoord::from_dvec3(DVec3::new(SECTOR_HALF + 1.0, 0.0, 0.0));
        assert_eq!(uc.sector, IVec3::new(1, 0, 0));
        // 513 - 1024 = -511.
        assert!((uc.offset.x - (-SECTOR_HALF + 1.0)).abs() < EPS);
    }

    #[test]
    fn negative_world_lands_in_negative_sector() {
        let world = DVec3::new(-1500.0, -200.0, 50.0);
        let uc = UniverseCoord::from_dvec3(world);
        // -1500 + 512 = -988; floor(-988 / 1024) = -1.
        assert_eq!(uc.sector.x, -1);
        // y = -200 stays in sector 0 (within half).
        assert_eq!(uc.sector.y, 0);
        // z = 50 stays in sector 0.
        assert_eq!(uc.sector.z, 0);
        assert!((uc.to_dvec3() - world).length() < EPS);
    }

    #[test]
    fn translate_across_sector_boundary() {
        let start = UniverseCoord::from_dvec3(DVec3::new(SECTOR_HALF - 1.0, 0.0, 0.0));
        assert_eq!(start.sector, IVec3::ZERO);
        let after = start.translated(DVec3::new(2.0, 0.0, 0.0));
        assert_eq!(after.sector, IVec3::new(1, 0, 0));
        assert!((after.offset.x - (-SECTOR_HALF + 1.0)).abs() < EPS);
    }

    #[test]
    fn translate_back_returns_to_origin_sector() {
        let start = UniverseCoord::ZERO;
        let mid = start.translated(DVec3::new(SECTOR_SIZE_METERS * 5.0, 0.0, 0.0));
        assert_eq!(mid.sector, IVec3::new(5, 0, 0));
        let back = mid.translated(DVec3::new(-SECTOR_SIZE_METERS * 5.0, 0.0, 0.0));
        assert_eq!(back.sector, IVec3::ZERO);
        assert!(back.offset.length() < EPS);
    }

    #[test]
    fn delta_to_preserves_precision_far_from_origin() {
        // Two coords 1 million meters from origin, 10 m apart.
        let a = UniverseCoord::from_dvec3(DVec3::new(1_000_000.0, 0.0, 0.0));
        let b = UniverseCoord::from_dvec3(DVec3::new(1_000_010.0, 0.0, 0.0));
        let delta = a.delta_to(&b);
        assert!((delta.x - 10.0).abs() < EPS);
        // f32 cast loses no precision because the delta is small.
        let delta_f32 = delta.as_vec3();
        assert!((delta_f32.x - 10.0).abs() < 1e-5);
    }

    #[test]
    fn normalised_idempotent() {
        // Build a non-canonical coord (offset outside the half range).
        let weird = UniverseCoord::new(IVec3::ZERO, DVec3::new(SECTOR_SIZE_METERS * 3.5, 0.0, 0.0));
        let canon = weird.normalised();
        // Canonical sector for world = 3584 m: 3584 + 512 = 4096; / 1024 = 4.0; floor = 4.
        assert_eq!(canon.sector.x, 4);
        // Offset = 3584 - 4096 = -512.
        assert!((canon.offset.x - (-SECTOR_HALF)).abs() < EPS);
        // Calling normalised again is a no-op.
        assert_eq!(canon, canon.normalised());
    }

    #[test]
    fn default_is_zero() {
        assert_eq!(UniverseCoord::default(), UniverseCoord::ZERO);
    }
}
