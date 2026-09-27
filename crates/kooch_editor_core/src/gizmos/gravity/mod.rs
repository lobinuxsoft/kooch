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

/// Fainter than [`EDGE`]: where the field has faded to nothing. Between the two shells is the
/// whole of `falloff`, which is the only way that number is visible at all.
const FADE: Vec3 = Vec3::new(0.22, 0.16, 0.38);

/// Long enough to read as a direction at a glance, short enough that a
/// handful of them do not fill the viewport.
pub(super) const ARROW: f32 = 1.5;

/// Draws an arrow of [`ARROW`] length from `base` along `direction` — the same solid arrow the
/// translate handle draws, so a field's pull reads like everything else that points somewhere.
fn arrow(gizmos: &mut Gizmos<'_>, base: Vec3, direction: Vec3, color: Vec3) {
    let Some(direction) = direction.try_normalize() else {
        return;
    };
    gizmos.arrow(
        base,
        base + direction * ARROW,
        glam::Vec4::new(color.x, color.y, color.z, 1.0),
    );
}
