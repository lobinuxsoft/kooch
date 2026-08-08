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
