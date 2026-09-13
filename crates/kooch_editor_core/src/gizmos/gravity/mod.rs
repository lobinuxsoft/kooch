//! Visualizers for the gravity sources — the worst case for an invisible component.

mod area;
mod box_field;
mod global;
mod plane;
mod point;

pub(crate) use area::AreaGravityVisualizer;
pub(crate) use box_field::BoxGravityVisualizer;
pub(crate) use global::GlobalGravityVisualizer;
pub(crate) use plane::PlaneGravityVisualizer;
pub(crate) use point::PointGravityVisualizer;

use glam::Vec3;

use kooch_gizmos::Gizmos;

/// Violet: unclaimed by colliders (green), lights (white), the centre of
/// mass (amber) or cameras (blue), so a field is never mistaken for the
/// geometry it passes through.
const FIELD: Vec3 = Vec3::new(0.62, 0.45, 0.98);

/// The same hue, darker, for a boundary that is a limit rather than the
/// field itself: a point source's cutoff, an area's falloff.
const EDGE: Vec3 = Vec3::new(0.36, 0.26, 0.60);

/// Long enough to read as a direction at a glance, short enough that a
/// handful of them do not fill the viewport.
pub(super) const ARROW: f32 = 1.5;

/// Draws an arrow of [`ARROW`] length from `base` along `direction`.
fn arrow(gizmos: &mut Gizmos<'_>, base: Vec3, direction: Vec3, color: Vec3) {
    let Some(direction) = direction.try_normalize() else {
        return;
    };
    let (a, b) = direction.any_orthonormal_pair();
    gizmos.arrow(base, base + direction * ARROW, a, b, color);
}
