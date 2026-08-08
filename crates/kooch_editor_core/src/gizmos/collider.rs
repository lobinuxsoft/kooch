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
//! [`BodySpec::desc`](kooch_physics::BodySpec) — box per axis, sphere on the
//! largest axis, capsule radius on its horizontal ones. If those two ever
//! disagree, the gizmo is worse than nothing: it would show a shape the
//! solver is not using. The same goes for [`Collider::center`]: the
//! outline is drawn at the shape's centre, because an outline at the
//! entity origin while the solver collides somewhere else is a lie in the
//! one place the tool exists to tell the truth.

use glam::{Mat3, Vec3};

use kooch_ecs::hierarchy::GlobalTransform;
use kooch_gizmos::{Gizmos, Visualizer};
use kooch_physics::components::{Collider, SHAPE_CAPSULE, SHAPE_CUBOID};

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
mod tests;
