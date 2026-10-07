//! A world size that holds a constant size **on screen** (#1433).
//!
//! 🔴 Every gizmo affordance used to be sized in world units, so it subtended an angle falling off
//! as `1/distance`: a 5 cm grip is sub-pixel at 50 m and fills the view at 50 cm. The affordance
//! only worked in the band of distances it was tuned for.
//!
//! ⚠️ This scales **where an affordance is drawn and picked, never the value it edits.** A tangent
//! grip sits at a third of its tangent and the drag divides by that third to recover the vector; if
//! the drawn offset became camera-relative while the stored tangent did not, dragging from far away
//! would scale the tangent by the camera distance.

use glam::Vec3;

use kooch_ecs::GlobalTransform;
use kooch_ecs::perspective_camera::PerspectiveCamera;

/// World units that cover one unit of the reference size at `at`.
///
/// `1.0` where there is no camera to ask, which leaves every caller at the world-unit sizing it had
/// before this existed.
pub(crate) fn factor(resources: &kooch_core::resource::Resources, at: Vec3) -> f32 {
    let Some((camera, transform)) = super::active_camera(resources) else {
        return 1.0;
    };
    of(&camera, &transform, at)
}

/// The same, for a camera already in hand — and the whole of the arithmetic, so a test can hold it
/// still without an ECS.
pub(crate) fn of(camera: &PerspectiveCamera, transform: &GlobalTransform, at: Vec3) -> f32 {
    let matrix = transform.matrix;
    let eye = matrix.w_axis.truncate();
    // 🔴 Along the view axis, not the straight-line distance. A handle at the edge of the frame is
    // further from the eye than one at the centre, so straight-line distance grows it as the camera
    // turns — the affordance would breathe while the view moves and nothing else changed.
    let forward = -matrix.z_axis.truncate().normalize_or(Vec3::NEG_Z);
    let along = (at - eye).dot(forward);

    // Behind the eye, or on it: nothing to size. Clamped rather than returned as zero so an
    // affordance that straddles the near plane keeps a usable size instead of collapsing.
    let along = along.max(MIN_DEPTH);
    // 🔴 `!(fov > 0.0)`, not `fov <= 0.0`: every comparison against NaN is false, and a NaN fov
    // would reach `tan` and hand back a NaN scale that puts the affordance nowhere.
    if !(camera.fov > 0.0) {
        return 1.0;
    }
    along * (camera.fov.to_radians() * 0.5).tan() * SPAN
}

/// Closest the sizing will measure from. An affordance at the eye would otherwise scale to nothing
/// and stop being clickable exactly when it is largest on screen.
const MIN_DEPTH: f32 = 0.05;

/// What one reference unit means, as a share of half the viewport's height.
///
/// Picked so the numbers callers already had keep their meaning: the transform arrows were 1.0 world
/// units and read well on a selection a few metres away, which is about a fifth of the half-height
/// at 60° — so a `factor` of 1 at that distance returns roughly 1.
const SPAN: f32 = 0.2;

#[cfg(test)]
mod tests;
