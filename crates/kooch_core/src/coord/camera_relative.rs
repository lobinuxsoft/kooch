//! Position relative to the active camera — what the GPU consumes.

use glam::Vec3;

use crate::coord::UniverseCoord;

/// Position relative to the active camera, in meters.
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

    /// Project an absolute `UniverseCoord` into the camera frame. The delta is computed in f64 (so
    /// positions millions of meters from the world origin remain accurate) and cast to f32 only
    /// after the subtraction — safe as long as `world` is within camera range.
    pub fn from_universe(world: UniverseCoord, camera: UniverseCoord) -> Self {
        let delta = camera.delta_to(&world);
        Self {
            position: delta.as_vec3(),
        }
    }

    /// Recover the absolute world position by adding the camera's
    /// universe coordinate back. Inverse of [`Self::from_universe`].
    pub fn to_universe(&self, camera: UniverseCoord) -> UniverseCoord {
        camera.translated(self.position.as_dvec3())
    }
}

#[cfg(test)]
mod tests;
