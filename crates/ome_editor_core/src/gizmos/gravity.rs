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

use glam::{Mat3, Vec3};

use ome_ecs::hierarchy::GlobalTransform;
use ome_gizmos::{Gizmos, Visualizer};
use ome_gravity::{AreaGravity, GlobalGravity, PointGravity};

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

/// A uniform field: parallel arrows, because one arrow is a vector and a
/// field is the same vector everywhere.
#[derive(Default)]
pub(crate) struct GlobalGravityVisualizer;

impl Visualizer<GlobalGravity> for GlobalGravityVisualizer {
    fn draw(&self, field: &GlobalGravity, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>) {
        let Some(direction) = field.acceleration.try_normalize() else {
            return;
        };
        let origin = transform.matrix.to_scale_rotation_translation().2;

        // World space on purpose, and the test that pins it: `acceleration`
        // is a world vector, so turning the entity must not turn the field.
        // Deriving the arrows from the entity's basis would be the obvious
        // thing and would quietly make the component lie.
        let (a, b) = direction.any_orthonormal_pair();
        const SPREAD: f32 = 0.8;
        for offset in [
            Vec3::ZERO,
            (a + b) * SPREAD,
            (a - b) * SPREAD,
            (-a + b) * SPREAD,
            (-a - b) * SPREAD,
        ] {
            arrow(gizmos, origin + offset, direction, FIELD);
        }
    }
}

/// A planet: the sphere where the strength is exact, the sphere where it
/// stops, and arrows saying the pull is inward.
#[derive(Default)]
pub(crate) struct PointGravityVisualizer;

impl Visualizer<PointGravity> for PointGravityVisualizer {
    fn draw(&self, field: &PointGravity, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>) {
        let (_, rotation, origin) = transform.matrix.to_scale_rotation_translation();
        let basis = Mat3::from_quat(rotation);

        // `radius` is where `strength` holds exactly — the one distance in
        // the component that means something an author can check.
        if field.radius > 0.0 {
            gizmos.wire_sphere(origin, basis, field.radius, FIELD);
        }
        // Zero or less is unlimited, and there is no sphere for infinity.
        if field.range > 0.0 {
            gizmos.wire_sphere(origin, basis, field.range, EDGE);
        }

        // Six arrows pointing *in*. This is the difference between a planet
        // and the world vector, and it is the whole reason #624 exists.
        let surface = match field.radius > 0.0 {
            true => field.radius,
            false => ARROW,
        };
        for axis in [
            Vec3::X,
            Vec3::NEG_X,
            Vec3::Y,
            Vec3::NEG_Y,
            Vec3::Z,
            Vec3::NEG_Z,
        ] {
            arrow(gizmos, origin + axis * surface, -axis, FIELD);
        }
    }
}

/// A room with its own down: the box, the reach of its falloff, and which
/// way down is inside it.
#[derive(Default)]
pub(crate) struct AreaGravityVisualizer;

impl Visualizer<AreaGravity> for AreaGravityVisualizer {
    fn draw(&self, field: &AreaGravity, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>) {
        let (scale, rotation, origin) = transform.matrix.to_scale_rotation_translation();
        let basis = Mat3::from_quat(rotation);
        // Scaled the way the field itself is: the box is authored in local
        // space, so a scaled entity has a bigger zone.
        let half = field.half_extents.abs() * scale;

        gizmos.wire_obb(origin, basis, half, FIELD);
        // The outer shell is where the field reaches zero. Without it the
        // author sees a hard box and cannot tell a 0.1 m fade from a 5 m one.
        if field.falloff > 0.0 {
            gizmos.wire_obb(origin, basis, half + Vec3::splat(field.falloff), EDGE);
        }

        // `direction` is local, so this is the drawing that shows a rotated
        // zone actually rotated — the exact thing that was invisible.
        let Some(local) = field.direction.try_normalize() else {
            return;
        };
        let world = rotation * local;
        // Four arrows spread across the face the field points away from, so
        // "down" reads as a direction through the volume rather than one
        // stray line at the centre.
        let (a, b) = local.any_orthonormal_pair();
        for (u, v) in [(0.5, 0.5), (0.5, -0.5), (-0.5, 0.5), (-0.5, -0.5)] {
            let inside = (a * u + b * v) * half;
            arrow(
                gizmos,
                origin + rotation * (inside - local * half * 0.5),
                world,
                FIELD,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Quat};
    use ome_gizmos::{GizmoBatch, MeshBatch};

    /// Every segment drawn, as `(start, end)` in world space.
    fn draw<C, V>(visualizer: &V, component: &C, matrix: Mat4) -> Vec<(Vec3, Vec3)>
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
    fn reach(segments: &[(Vec3, Vec3)]) -> f32 {
        segments
            .iter()
            .flat_map(|(a, b)| [a.length(), b.length()])
            .fold(0.0, f32::max)
    }

    /// The direction the longest segments run in, which for an arrow shaft
    /// is the direction of the field.
    fn shaft(segments: &[(Vec3, Vec3)]) -> Vec3 {
        segments
            .iter()
            .max_by(|x, y| (x.1 - x.0).length().total_cmp(&(y.1 - y.0).length()))
            .map(|(a, b)| (*b - *a).normalize())
            .expect("nothing was drawn")
    }

    #[test]
    fn a_uniform_field_draws_along_its_acceleration() {
        let field = GlobalGravity::default();
        let segments = draw(&GlobalGravityVisualizer, &field, Mat4::IDENTITY);
        assert!(shaft(&segments).abs_diff_eq(Vec3::NEG_Y, 1e-3));
    }

    /// `acceleration` is a world vector. Deriving the arrows from the
    /// entity's basis would be the natural thing to write and would make
    /// the gizmo disagree with the solver the moment anyone rotated it.
    #[test]
    fn a_uniform_field_does_not_turn_with_its_entity() {
        let field = GlobalGravity::default();
        let turned = Mat4::from_quat(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
        assert!(
            shaft(&draw(&GlobalGravityVisualizer, &field, turned)).abs_diff_eq(Vec3::NEG_Y, 1e-3)
        );
    }

    #[test]
    fn a_degenerate_field_draws_nothing() {
        let field = GlobalGravity {
            acceleration: Vec3::ZERO,
        };
        assert!(draw(&GlobalGravityVisualizer, &field, Mat4::IDENTITY).is_empty());
    }

    /// The cutoff is the outer sphere, so the gizmo has to reach it.
    #[test]
    fn a_point_source_draws_out_to_its_range() {
        let field = PointGravity {
            radius: 10.0,
            range: 100.0,
            ..Default::default()
        };
        let reach = reach(&draw(&PointGravityVisualizer, &field, Mat4::IDENTITY));
        assert!((reach - 100.0).abs() < 1.0, "reached {reach}, wanted 100");
    }

    /// Zero range means unlimited, and there is no sphere for infinity —
    /// so the drawing must stop at the radius rather than at zero.
    #[test]
    fn an_unlimited_point_source_draws_only_its_radius() {
        let field = PointGravity {
            radius: 10.0,
            range: 0.0,
            ..Default::default()
        };
        let reach = reach(&draw(&PointGravityVisualizer, &field, Mat4::IDENTITY));
        assert!((reach - 10.0).abs() < 1.0, "reached {reach}, wanted 10");
    }

    /// A planet pulls inward. If the arrows pointed out it would read as a
    /// repulsor, which is the one thing this component is not.
    #[test]
    fn a_point_source_points_inward() {
        let field = PointGravity {
            radius: 10.0,
            range: 0.0,
            ..Default::default()
        };
        let segments = draw(&PointGravityVisualizer, &field, Mat4::IDENTITY);
        let shafts: Vec<_> = segments
            .iter()
            .filter(|(a, b)| ((*b - *a).length() - ARROW).abs() < 1e-3)
            .collect();
        assert_eq!(shafts.len(), 6, "expected one arrow per axis");
        for (a, b) in shafts {
            assert!(
                b.length() < a.length(),
                "an arrow from {a} to {b} points away from the centre",
            );
        }
    }

    #[test]
    fn an_area_draws_its_box() {
        let field = AreaGravity {
            half_extents: Vec3::splat(5.0),
            falloff: 0.0,
            ..Default::default()
        };
        let corner = Vec3::splat(5.0).length();
        let reach = reach(&draw(&AreaGravityVisualizer, &field, Mat4::IDENTITY));
        assert!(
            (reach - corner).abs() < 0.1,
            "reached {reach}, wanted {corner}"
        );
    }

    /// The falloff is where the field actually ends, and it is invisible
    /// otherwise: a 0.1 m fade and a 5 m fade look identical without it.
    #[test]
    fn an_area_draws_the_reach_of_its_falloff() {
        let with = AreaGravity {
            half_extents: Vec3::splat(5.0),
            falloff: 5.0,
            ..Default::default()
        };
        let without = AreaGravity {
            falloff: 0.0,
            ..with
        };
        assert!(
            reach(&draw(&AreaGravityVisualizer, &with, Mat4::IDENTITY))
                > reach(&draw(&AreaGravityVisualizer, &without, Mat4::IDENTITY)) + 1.0,
        );
    }

    /// `direction` is local, so a rotated zone must draw rotated. This is
    /// the case that had no way of being seen and dropped things sideways
    /// for a reason nobody could point at.
    #[test]
    fn an_area_turns_with_its_entity() {
        let field = AreaGravity::default();
        let turned = Mat4::from_quat(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
        let segments = draw(&AreaGravityVisualizer, &field, turned);
        let shafts: Vec<Vec3> = segments
            .iter()
            .filter(|(a, b)| ((*b - *a).length() - ARROW).abs() < 1e-3)
            .map(|(a, b)| (*b - *a).normalize())
            .collect();

        assert!(!shafts.is_empty(), "no arrows were drawn");
        // Local -Y turned a quarter turn about +Z points along +X, the same
        // answer `plugin::gravity_at` gives for the same transform.
        for shaft in shafts {
            assert!(
                shaft.abs_diff_eq(Vec3::X, 1e-3),
                "a rotated zone drew its arrow along {shaft}",
            );
        }
    }

    /// A scaled entity has a bigger zone, the same way the solver scales it.
    #[test]
    fn an_area_scales_with_its_entity() {
        let field = AreaGravity {
            half_extents: Vec3::splat(5.0),
            falloff: 0.0,
            ..Default::default()
        };
        let plain = reach(&draw(&AreaGravityVisualizer, &field, Mat4::IDENTITY));
        let scaled = reach(&draw(
            &AreaGravityVisualizer,
            &field,
            Mat4::from_scale(Vec3::splat(2.0)),
        ));
        assert!((scaled / plain - 2.0).abs() < 0.05, "{plain} then {scaled}");
    }
}
