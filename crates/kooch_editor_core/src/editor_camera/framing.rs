//! Where the camera goes when you press F.

use glam::Vec3;

/// What F should frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FocusTarget {
    pub point: Vec3,
    /// Radius of the sphere that has to fit on screen, or `None` when
    /// nothing says how big the thing is.
    pub radius: Option<f32>,
}

/// How far back a camera has to sit for a sphere of `radius` to fill its view.
pub fn distance_for(radius: Option<f32>, fov: f32) -> f32 {
    const NO_MESH: f32 = 5.0;
    /// Closer than this and the near plane starts clipping what you
    /// just asked to look at.
    const NEAREST: f32 = 0.35;

    let Some(radius) = radius.filter(|r| *r > 0.0) else {
        return NO_MESH;
    };
    let half = (fov * 0.5).clamp(0.01, std::f32::consts::FRAC_PI_2 - 0.01);
    (radius / half.sin()).max(NEAREST)
}

/// The radius of the sphere around `centre` that contains the box.
pub fn radius_around(centre: Vec3, min: Vec3, max: Vec3) -> f32 {
    (centre - min).length().max((max - centre).length())
}

#[cfg(test)]
mod tests;
