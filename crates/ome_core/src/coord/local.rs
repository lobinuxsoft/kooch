//! Position relative to a celestial body (planet, moon, station).
//!
//! Once the engine has celestial bodies as ECS entities, every gameplay
//! position is naturally expressed relative to one of them. f32 precision
//! is sufficient within a single planet's bounds (R ≈ 6000 km gives
//! ~1 cm at the antipode — more than enough for gameplay), so this is
//! the level the rest of the engine consumes most of the time.

use glam::Vec3;

use crate::coord::UniverseCoord;

/// Opaque reference to the celestial body that a [`LocalCoord`] is
/// relative to. Stored as `u64` to avoid a circular dependency between
/// `ome_core` and `ome_ecs`; `ome_ecs` will provide `From<Entity>` /
/// `Into<Entity>` conversions in its own coord-extension module so
/// callsites can stay ergonomic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct CelestialBodyRef(pub u64);

impl CelestialBodyRef {
    /// Sentinel meaning "no body" — used for free-floating coordinates
    /// in interplanetary space, or as a placeholder before the celestial
    /// body system spawns its first body.
    pub const NONE: Self = Self(0);

    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }
}

/// Position relative to a celestial body, in meters.
///
/// Use [`Self::from_universe`] to project an absolute [`UniverseCoord`]
/// into the local frame of a body, and [`Self::to_universe`] to recover
/// the absolute position.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct LocalCoord {
    /// The body this position is relative to.
    pub reference: CelestialBodyRef,
    /// Position in meters from the body's origin (typically the body
    /// centre for a planet, or its anchor point for a station).
    pub position: Vec3,
}

impl LocalCoord {
    /// Project a `UniverseCoord` into the local frame of a body whose
    /// origin sits at `body_origin`. The delta is computed in f64 and
    /// cast to f32 — safe as long as `world` is within a body's bounds
    /// (a few thousand km), which is the use case this type targets.
    pub fn from_universe(
        world: UniverseCoord,
        body_origin: UniverseCoord,
        reference: CelestialBodyRef,
    ) -> Self {
        let delta = body_origin.delta_to(&world);
        Self {
            reference,
            position: delta.as_vec3(),
        }
    }

    /// Recover the absolute world position by adding the body's origin
    /// back. Inverse of [`Self::from_universe`].
    pub fn to_universe(&self, body_origin: UniverseCoord) -> UniverseCoord {
        body_origin.translated(self.position.as_dvec3())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    const EPS_F32: f32 = 1e-3;
    const EPS_F64: f64 = 1e-3;

    #[test]
    fn celestial_body_ref_default_is_none() {
        assert_eq!(CelestialBodyRef::default(), CelestialBodyRef::NONE);
        assert_eq!(CelestialBodyRef::NONE.0, 0);
    }

    #[test]
    fn local_at_body_origin_is_zero() {
        let body = UniverseCoord::from_dvec3(DVec3::new(123.0, 456.0, 789.0));
        let local = LocalCoord::from_universe(body, body, CelestialBodyRef::NONE);
        assert!(local.position.length() < EPS_F32);
    }

    #[test]
    fn round_trip_through_local() {
        // Body 1 million meters from origin; world point 10 m offset.
        let body = UniverseCoord::from_dvec3(DVec3::new(1_000_000.0, 0.0, 0.0));
        let world = UniverseCoord::from_dvec3(DVec3::new(1_000_010.0, 5.0, -3.0));

        let local = LocalCoord::from_universe(world, body, CelestialBodyRef::new(42));
        assert_eq!(local.reference.0, 42);
        assert!((local.position - Vec3::new(10.0, 5.0, -3.0)).length() < EPS_F32);

        let back = local.to_universe(body);
        assert!((back.to_dvec3() - world.to_dvec3()).length() < EPS_F64);
    }

    #[test]
    fn ref_propagates_through_round_trip() {
        let body = UniverseCoord::ZERO;
        let world = UniverseCoord::from_dvec3(DVec3::new(7.0, 0.0, 0.0));
        let r = CelestialBodyRef::new(0xCAFE_BABE);
        let local = LocalCoord::from_universe(world, body, r);
        assert_eq!(local.reference, r);
    }

    #[test]
    fn far_body_preserves_local_precision() {
        // Body 100 km from origin — well past the f32 precision cliff
        // (5 km). Local-frame position should still resolve cleanly.
        let body = UniverseCoord::from_dvec3(DVec3::new(100_000.0, 0.0, 0.0));
        let world = UniverseCoord::from_dvec3(DVec3::new(100_000.5, 0.0, 0.0));
        let local = LocalCoord::from_universe(world, body, CelestialBodyRef::NONE);
        assert!((local.position.x - 0.5).abs() < EPS_F32);
    }
}
