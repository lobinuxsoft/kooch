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
mod tests;
