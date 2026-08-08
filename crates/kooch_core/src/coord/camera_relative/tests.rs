use super::*;
use crate::coord::CelestialBodyRef;
use glam::DVec3;

const EPS_F32: f32 = 1e-3;
const EPS_F64: f64 = 1e-3;

#[test]
fn camera_at_world_returns_world_position() {
    // When the camera sits at the universe origin, camera-relative
    // == absolute (within the f32 representable range).
    let cam = UniverseCoord::ZERO;
    let world = UniverseCoord::from_dvec3(DVec3::new(10.0, 5.0, -3.0));
    let cr = CameraRelativeCoord::from_universe(world, cam);
    assert!((cr.position - Vec3::new(10.0, 5.0, -3.0)).length() < EPS_F32);
}

#[test]
fn camera_at_subject_yields_zero() {
    let cam = UniverseCoord::from_dvec3(DVec3::new(123.0, 456.0, 789.0));
    let cr = CameraRelativeCoord::from_universe(cam, cam);
    assert!(cr.position.length() < EPS_F32);
}

#[test]
fn round_trip_universe_camera_universe() {
    let cam = UniverseCoord::from_dvec3(DVec3::new(1_000_000.0, 0.0, 0.0));
    let world = UniverseCoord::from_dvec3(DVec3::new(1_000_007.5, 2.5, -1.25));
    let cr = CameraRelativeCoord::from_universe(world, cam);
    let back = cr.to_universe(cam);
    assert!((back.to_dvec3() - world.to_dvec3()).length() < EPS_F64);
}

#[test]
fn far_camera_preserves_near_object_precision() {
    // Camera 1 million meters from the origin; an object 10 meters
    // in front of it must still resolve at f32 sub-mm precision.
    let cam = UniverseCoord::from_dvec3(DVec3::new(1_000_000.0, 0.0, 0.0));
    let world = UniverseCoord::from_dvec3(DVec3::new(1_000_010.0, 0.0, 0.0));
    let cr = CameraRelativeCoord::from_universe(world, cam);
    // f32 absolute error at 10.0 is ~1e-6 — well within EPS_F32.
    assert!((cr.position.x - 10.0).abs() < 1e-5);
}

#[test]
fn from_local_composes_through_universe() {
    // Body 1 km from origin; camera 100 m from body; world point
    // 5 m from body.
    let body_origin = UniverseCoord::from_dvec3(DVec3::new(1000.0, 0.0, 0.0));
    let cam = UniverseCoord::from_dvec3(DVec3::new(1100.0, 0.0, 0.0));
    let local = LocalCoord {
        reference: CelestialBodyRef::new(7),
        position: Vec3::new(5.0, 0.0, 0.0),
    };
    let cr = CameraRelativeCoord::from_local(local, body_origin, cam);
    // World = body + 5 = 1005. Camera = 1100. Delta = -95.
    assert!((cr.position.x - (-95.0)).abs() < EPS_F32);
}

#[test]
fn zero_default() {
    assert_eq!(CameraRelativeCoord::default(), CameraRelativeCoord::ZERO);
    assert_eq!(CameraRelativeCoord::ZERO.position, Vec3::ZERO);
}
