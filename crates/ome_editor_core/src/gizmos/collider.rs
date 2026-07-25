//! [`ColliderVisualizer`] — draws what the solver will actually collide
//! against.
//!
//! A collider used to be invisible: authored as `radius` and
//! `half_extents` in the Inspector, with no way to see whether the shape
//! wrapped the model. Getting it wrong stayed silent until something fell
//! through the floor or hovered above it — and it hid a real bug, where
//! the collider ignored `Transform.scale` and only matched its mesh at the
//! size it was authored.
//!
//! # It reads the scale the same way the solver does
//!
//! The outline has to show the *effective* shape, not the authored
//! numbers, or it lies exactly where a collider is most likely to be
//! wrong. So the scale folding here mirrors
//! [`BodySpec::desc`](ome_physics::BodySpec) — box per axis, sphere on the
//! largest axis, capsule radius on its horizontal ones. If those two ever
//! disagree, the gizmo is worse than nothing: it would show a shape the
//! solver is not using. The same goes for [`Collider::center`]: the
//! outline is drawn at the shape's centre, because an outline at the
//! entity origin while the solver collides somewhere else is a lie in the
//! one place the tool exists to tell the truth.

use glam::{Mat3, Vec3};

use ome_ecs::hierarchy::GlobalTransform;
use ome_gizmos::{Gizmos, Visualizer};
use ome_physics::components::{Collider, SHAPE_CAPSULE, SHAPE_CUBOID};

/// Wireframe colour for a solid collider.
///
/// Green, the convention in Unity, Unreal and Godot — worth matching
/// rather than inventing, because it is the one thing about a collider
/// gizmo that everyone already knows.
const SOLID: Vec3 = Vec3::new(0.35, 0.95, 0.4);

#[derive(Default)]
pub(crate) struct ColliderVisualizer;

impl Visualizer<Collider> for ColliderVisualizer {
    fn draw(&self, collider: &Collider, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>) {
        let (scale, rotation, translation) = transform.matrix.to_scale_rotation_translation();
        let basis = Mat3::from_quat(rotation);
        let s = scale.abs();
        // Drawn at the shape's centre, not the entity's origin. An outline
        // at the origin while the solver collides half a metre up would
        // actively mislead — worse than no outline at all, which is the
        // whole point of drawing one.
        let translation = translation + rotation * (collider.center * s);

        match collider.shape {
            SHAPE_CUBOID => {
                // A box is the one shape that scales exactly, per axis.
                gizmos.wire_obb(translation, basis, collider.half_extents * s, SOLID);
            }
            SHAPE_CAPSULE => {
                // Radius follows the horizontal axes: a capsule scaled on
                // Y gets taller, not fatter.
                gizmos.wire_capsule(
                    translation,
                    basis,
                    collider.radius * s.x.max(s.z),
                    collider.half_height * s.y,
                    SOLID,
                );
            }
            // Sphere is the default for an unknown discriminant, matching
            // `Collider::collision_shape` — a scene from a newer editor
            // draws something rather than nothing.
            _ => {
                gizmos.wire_sphere(translation, basis, collider.radius * s.max_element(), SOLID);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Quat};
    use ome_gizmos::{GizmoBatch, MeshBatch};

    /// Draws one collider and returns the line segments produced.
    fn draw(collider: &Collider, matrix: Mat4) -> Vec<(Vec3, Vec3)> {
        let mut lines = GizmoBatch::default();
        let mut meshes = MeshBatch::default();
        {
            let mut gizmos = Gizmos::new(&mut lines, &mut meshes);
            ColliderVisualizer.draw(collider, &GlobalTransform { matrix }, &mut gizmos);
        }
        lines.lines.iter().map(|s| (s.start, s.end)).collect()
    }

    /// The furthest any drawn point sits from `centre`, per axis.
    fn extent(lines: &[(Vec3, Vec3)], centre: Vec3) -> Vec3 {
        lines
            .iter()
            .flat_map(|(a, b)| [*a, *b])
            .fold(Vec3::ZERO, |acc, p| acc.max((p - centre).abs()))
    }

    #[test]
    fn a_collider_draws_something() {
        for shape in [SHAPE_CUBOID, SHAPE_CAPSULE, u32::MAX] {
            let collider = Collider {
                shape,
                ..Default::default()
            };
            assert!(
                !draw(&collider, Mat4::IDENTITY).is_empty(),
                "shape {shape} drew nothing"
            );
        }
    }

    /// The outline follows the *effective* shape, scale included. This is
    /// the assertion that matters: an outline drawn from the authored
    /// numbers would lie precisely where a collider is most likely wrong.
    #[test]
    fn the_outline_grows_with_the_transform_scale() {
        let collider = Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::splat(0.5),
            ..Default::default()
        };
        let unscaled = extent(&draw(&collider, Mat4::IDENTITY), Vec3::ZERO);
        let scaled = extent(
            &draw(
                &collider,
                Mat4::from_scale_rotation_translation(
                    Vec3::new(3.0, 1.0, 5.0),
                    Quat::IDENTITY,
                    Vec3::ZERO,
                ),
            ),
            Vec3::ZERO,
        );

        assert!(unscaled.abs_diff_eq(Vec3::splat(0.5), 1e-4), "{unscaled:?}");
        assert!(
            scaled.abs_diff_eq(Vec3::new(1.5, 0.5, 2.5), 1e-4),
            "the outline ignored the transform scale: {scaled:?}"
        );
    }

    /// A sphere takes the largest axis, exactly as the solver does.
    #[test]
    fn a_scaled_sphere_outline_takes_the_largest_axis() {
        let extent = extent(
            &draw(
                &Collider::default(),
                Mat4::from_scale_rotation_translation(
                    Vec3::new(1.0, 4.0, 2.0),
                    Quat::IDENTITY,
                    Vec3::ZERO,
                ),
            ),
            Vec3::ZERO,
        );
        // 0.5 * 4.0 = 2.0 on every axis: a sphere, not an ellipsoid.
        assert!(extent.min_element() > 1.9, "got {extent:?}");
        assert!(extent.max_element() < 2.1, "got {extent:?}");
    }

    /// A capsule scaled on Y gets taller, not fatter.
    #[test]
    fn a_scaled_capsule_outline_grows_along_its_own_axis() {
        let collider = Collider {
            shape: SHAPE_CAPSULE,
            radius: 0.5,
            half_height: 1.0,
            ..Default::default()
        };
        let extent = extent(
            &draw(
                &collider,
                Mat4::from_scale_rotation_translation(
                    Vec3::new(1.0, 3.0, 1.0),
                    Quat::IDENTITY,
                    Vec3::ZERO,
                ),
            ),
            Vec3::ZERO,
        );
        // half_height 3.0 plus radius 0.5 = 3.5 tall, still 0.5 wide.
        assert!((extent.y - 3.5).abs() < 0.05, "height wrong: {extent:?}");
        assert!((extent.x - 0.5).abs() < 0.05, "it got fatter: {extent:?}");
    }

    /// The outline is drawn where the entity is, not at the origin.
    #[test]
    fn the_outline_follows_the_entity() {
        let centre = Vec3::new(10.0, -4.0, 7.0);
        let lines = draw(&Collider::default(), Mat4::from_translation(centre));
        let extent = extent(&lines, centre);
        assert!(
            extent.max_element() < 0.6,
            "the outline is not centred on the entity: {extent:?}"
        );
    }

    /// A rotated entity's outline rotates with it. Drawing it
    /// axis-aligned would be worse than drawing nothing — it would look
    /// like the collider had not rotated.
    #[test]
    fn the_outline_rotates_with_the_entity() {
        let collider = Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(2.0, 0.1, 0.1),
            ..Default::default()
        };
        let flat = extent(&draw(&collider, Mat4::IDENTITY), Vec3::ZERO);
        assert!(flat.x > 1.9 && flat.z < 0.2, "{flat:?}");

        // A quarter turn about Y swaps the long axis from X to Z.
        let turned = extent(
            &draw(
                &collider,
                Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2),
            ),
            Vec3::ZERO,
        );
        assert!(
            turned.z > 1.9 && turned.x < 0.2,
            "the outline did not rotate: {turned:?}"
        );
    }

    /// The outline follows `Collider.center`, so what you see is where the
    /// solver collides.
    #[test]
    fn the_outline_sits_at_the_shape_centre() {
        let collider = Collider {
            center: Vec3::new(0.0, 2.0, 0.0),
            ..Default::default()
        };
        let lines = draw(&collider, Mat4::IDENTITY);

        // Tight around the offset centre, and nothing near the origin.
        assert!(
            extent(&lines, Vec3::new(0.0, 2.0, 0.0)).max_element() < 0.6,
            "the outline is not centred on the shape"
        );
        let lowest = lines
            .iter()
            .flat_map(|(a, b)| [a.y, b.y])
            .fold(f32::INFINITY, f32::min);
        assert!(
            lowest > 1.4,
            "the outline reaches down to the entity origin: lowest y = {lowest}"
        );
    }

    /// The offset rotates with the entity, and scales with it.
    #[test]
    fn the_shape_centre_follows_the_transform() {
        let collider = Collider {
            center: Vec3::new(0.0, 1.0, 0.0),
            ..Default::default()
        };

        // A half turn about X sends a +Y offset to -Y.
        let turned = draw(&collider, Mat4::from_rotation_x(std::f32::consts::PI));
        assert!(
            extent(&turned, Vec3::new(0.0, -1.0, 0.0)).max_element() < 0.6,
            "the offset did not rotate with the entity"
        );

        // Scale multiplies the offset along with the dimensions.
        let scaled = draw(
            &collider,
            Mat4::from_scale_rotation_translation(Vec3::splat(3.0), Quat::IDENTITY, Vec3::ZERO),
        );
        assert!(
            extent(&scaled, Vec3::new(0.0, 3.0, 0.0)).max_element() < 1.7,
            "the offset did not scale with the entity"
        );
    }
}
