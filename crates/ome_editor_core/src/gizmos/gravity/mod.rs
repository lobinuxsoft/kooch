//! Visualizers for the gravity sources — the worst case for an invisible
//! component.
//!
//! A collider that is the wrong size shows up the moment something rests
//! on it. A gravity field has no surface, no mesh, and no contact: an
//! `AreaGravity` rotated ninety degrees looks exactly like one that is not,
//! until something falls sideways and the author has to guess why. Every
//! number these components carry — a radius, a range, a box, a local
//! direction — is a piece of world geometry that nothing else draws.
//!
//! # Direction is drawn, magnitude is not
//!
//! An arrow scaled by `strength` would be 9.81 units long for ordinary
//! gravity, which is a building. These arrows are a fixed length and say
//! only which way the field pulls. The strength is a number in the
//! Inspector, and a number is a perfectly good way to read a number.

mod area;
mod box_field;
mod global;
mod point;

pub(crate) use area::AreaGravityVisualizer;
pub(crate) use box_field::BoxGravityVisualizer;
pub(crate) use global::GlobalGravityVisualizer;
pub(crate) use point::PointGravityVisualizer;

use glam::Vec3;

use ome_gizmos::Gizmos;

/// Violet: unclaimed by colliders (green), lights (white), the centre of
/// mass (amber) or cameras (blue), so a field is never mistaken for the
/// geometry it passes through.
const FIELD: Vec3 = Vec3::new(0.62, 0.45, 0.98);

/// The same hue, darker, for a boundary that is a limit rather than the
/// field itself: a point source's cutoff, an area's falloff.
const EDGE: Vec3 = Vec3::new(0.36, 0.26, 0.60);

/// Long enough to read as a direction at a glance, short enough that a
/// handful of them do not fill the viewport.
const ARROW: f32 = 1.5;

/// Draws an arrow of [`ARROW`] length from `base` along `direction`.
///
/// The perpendiculars for the head are derived rather than passed: a
/// gravity arrow has no roll anyone can observe, so any pair will do.
fn arrow(gizmos: &mut Gizmos<'_>, base: Vec3, direction: Vec3, color: Vec3) {
    let Some(direction) = direction.try_normalize() else {
        return;
    };
    let (a, b) = direction.any_orthonormal_pair();
    gizmos.arrow(base, base + direction * ARROW, a, b, color);
}

#[cfg(test)]
mod harness {
    use super::*;
    use glam::Mat4;
    use ome_ecs::hierarchy::GlobalTransform;
    use ome_gizmos::{GizmoBatch, MeshBatch, Visualizer};

    /// Every segment drawn, as `(start, end)` in world space.
    pub(super) fn draw<C, V>(visualizer: &V, component: &C, matrix: Mat4) -> Vec<(Vec3, Vec3)>
    where
        V: Visualizer<C>,
        C: ome_ecs::component::Component,
    {
        let mut lines = GizmoBatch::default();
        let mut meshes = MeshBatch::default();
        let mut gizmos = Gizmos::new(&mut lines, &mut meshes);
        visualizer.draw(component, &GlobalTransform { matrix }, &mut gizmos);
        lines.lines.iter().map(|s| (s.start, s.end)).collect()
    }

    /// The furthest any drawn point gets from the origin — how far the
    /// gizmo claims the field reaches.
    pub(super) fn reach(segments: &[(Vec3, Vec3)]) -> f32 {
        segments
            .iter()
            .flat_map(|(a, b)| [a.length(), b.length()])
            .fold(0.0, f32::max)
    }

    /// The direction the longest segments run in, which for an arrow shaft
    /// is the direction of the field.
    pub(super) fn shaft(segments: &[(Vec3, Vec3)]) -> Vec3 {
        segments
            .iter()
            .max_by(|x, y| (x.1 - x.0).length().total_cmp(&(y.1 - y.0).length()))
            .map(|(a, b)| (*b - *a).normalize())
            .expect("nothing was drawn")
    }

    /// Only the arrow shafts, as unit directions — the heads are short
    /// segments at the tip and would drown the signal.
    pub(super) fn shafts(segments: &[(Vec3, Vec3)]) -> Vec<Vec3> {
        segments
            .iter()
            .filter(|(a, b)| ((*b - *a).length() - ARROW).abs() < 1e-3)
            .map(|(a, b)| (*b - *a).normalize())
            .collect()
    }
}
