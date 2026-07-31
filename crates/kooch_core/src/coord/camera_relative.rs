//! Position relative to the active camera — what the GPU consumes.
//!
//! Cameras-relative coordinates are always near zero (within view
//! distance + frustum), so f32 has full precision regardless of how
//! far the camera has travelled in the universe. This is the
//! coordinate frame uploaded to the shader every frame.

use glam::Vec3;

use crate::coord::{LocalCoord, UniverseCoord};

/// Position relative to the active camera, in meters.
///
/// The shader pipeline consumes [`CameraRelativeCoord`] for vertex
/// transforms, SDF primitive positions, sky direction, etc. — anything
/// that the GPU evaluates in screen / clip space.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct CameraRelativeCoord {
    pub position: Vec3,
}

impl CameraRelativeCoord {
    pub const ZERO: Self = Self {
        position: Vec3::ZERO,
    };

    pub const fn new(position: Vec3) -> Self {
        Self { position }
    }

    /// Project an absolute `UniverseCoord` into the camera frame. The
    /// delta is computed in f64 (so positions millions of meters from
    /// the world origin remain accurate) and cast to f32 only after the
    /// subtraction — safe as long as `world` is within camera range.
    pub fn from_universe(world: UniverseCoord, camera: UniverseCoord) -> Self {
        let delta = camera.delta_to(&world);
        Self {
            position: delta.as_vec3(),
        }
    }

    /// Compose a `LocalCoord` (already relative to a celestial body)
    /// into camera-relative form. Goes via [`UniverseCoord`] internally
    /// so precision is preserved when the body itself is far from the
    /// camera.
    pub fn from_local(
        local: LocalCoord,
        body_origin: UniverseCoord,
        camera: UniverseCoord,
    ) -> Self {
        let world = local.to_universe(body_origin);
        Self::from_universe(world, camera)
    }

    /// Recover the absolute world position by adding the camera's
    /// universe coordinate back. Inverse of [`Self::from_universe`].
    pub fn to_universe(&self, camera: UniverseCoord) -> UniverseCoord {
        camera.translated(self.position.as_dvec3())
    }
}

#[cfg(test)]
mod tests {
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
}
