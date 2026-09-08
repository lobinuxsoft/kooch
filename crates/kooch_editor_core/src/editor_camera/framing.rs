//! Where the camera goes when you press F.

use glam::Vec3;

/// What F should frame.
///
/// A point alone centres the view and leaves it wherever it was, which
/// is what F did: a block a hundred metres away became a block a
/// hundred metres away in the middle of the screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FocusTarget {
    pub point: Vec3,
    /// Radius of the sphere that has to fit on screen, or `None` when
    /// nothing says how big the thing is.
    pub radius: Option<f32>,
}

/// How far back a camera has to sit for a sphere of `radius` to fill
/// its view.
///
/// `d = r / sin(fov / 2)` — the sphere is tangent to the frustum, so
/// its silhouette touches the top and bottom edges. Uses the **vertical**
/// fov because that is the one that does not change with the panel's
/// aspect: framing that depended on how wide the viewport was dragged
/// would put the same block at a different size in two docks.
///
/// # Without a radius
///
/// Five units. An entity with a transform and no mesh is a spawn point,
/// a trigger, a camera — things whose gizmos are drawn at a fixed size
/// and read well from about there. Framing "nothing" has no correct
/// answer, and standing still was the worse one.
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
///
/// The corner distance, not half the diagonal from the centre of the
/// box: a face selection's centre is not its bounding box's centre, and
/// using the box's own would frame a point the camera is not aiming at.
pub fn radius_around(centre: Vec3, min: Vec3, max: Vec3) -> f32 {
    (centre - min).length().max((max - centre).length())
}

#[cfg(test)]
mod tests;
