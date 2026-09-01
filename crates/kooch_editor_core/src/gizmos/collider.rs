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
//!
//! # The mesh-derived shapes draw nothing
//!
//! A hull or a trimesh outline needs the vertices, which live in
//! [`ColliderMeshCache`] — and a [`Visualizer`] is handed a component and
//! a transform, not `Resources`. Drawing a sphere instead would be the
//! same lie this file exists to avoid, so those draw nothing until #574
//! widens the contract.
//!
//! [`ColliderMeshCache`]: kooch_physics::ColliderMeshCache

use glam::{Mat3, Vec3};

use kooch_ecs::hierarchy::GlobalTransform;
use kooch_gizmos::{Gizmos, Visualizer};
use kooch_physics::components::{
    Collider, SHAPE_CAPSULE, SHAPE_CONE, SHAPE_CUBOID, SHAPE_CYLINDER, SHAPE_HALF_SPACE,
    SHAPE_ROUND_CYLINDER, SHAPE_SEGMENT, SHAPE_TRIANGLE, is_mesh_derived,
};

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
        // Corners are authored in the shape's own space, so they start
        // from its centre rather than from the entity's origin.
        let point = |local: Vec3| translation + rotation * (local * s);

        // A hull's outline is its mesh's, and a visualizer cannot reach
        // the cache that holds it. Nothing beats a sphere that is not
        // what the solver collides against.
        if is_mesh_derived(collider.shape) {
            return;
        }

        // Radius follows the horizontal axes on everything aligned to Y:
        // scaled on Y they get taller, not fatter.
        let flat = collider.radius * s.x.max(s.z);
        let tall = collider.half_height * s.y;
        match collider.shape {
            SHAPE_CUBOID => {
                // A box is the one shape that scales exactly, per axis.
                gizmos.wire_obb(translation, basis, collider.half_extents * s, SOLID);
            }
            SHAPE_CAPSULE => gizmos.wire_capsule(translation, basis, flat, tall, SOLID),
            // The fillet is a fraction of the radius and would read as a
            // wobble at gizmo line width, so both cylinders draw the same.
            SHAPE_CYLINDER | SHAPE_ROUND_CYLINDER => {
                gizmos.wire_cylinder(translation, basis, flat, tall, SOLID);
            }
            SHAPE_CONE => gizmos.wire_cone(translation, basis, flat, tall, SOLID),
            // Sized off the shape rather than the view: a patch that
            // resized with the camera would read as geometry that moves.
            SHAPE_HALF_SPACE => {
                gizmos.wire_halfspace(translation, rotation * collider.normal, PLANE_PATCH, SOLID);
            }
            SHAPE_SEGMENT => {
                gizmos.line(point(collider.point_a), point(collider.point_b), SOLID);
            }
            SHAPE_TRIANGLE => {
                let (a, b, c) = (
                    point(collider.point_a),
                    point(collider.point_b),
                    point(collider.point_c),
                );
                gizmos.line(a, b, SOLID);
                gizmos.line(b, c, SOLID);
                gizmos.line(c, a, SOLID);
            }
            // Sphere for the sphere and for an unknown discriminant,
            // matching `ShapeSpec::resolve` — a scene from a newer editor
            // draws something rather than nothing.
            _ => {
                gizmos.wire_sphere(translation, basis, collider.radius * s.max_element(), SOLID);
            }
        }
    }
}

/// How far the half-space patch reaches from the shape's centre.
///
/// An infinite plane cannot be drawn; this is the suggestion of one, big
/// enough to read as ground and small enough not to swallow the scene.
const PLANE_PATCH: f32 = 5.0;

#[cfg(test)]
mod tests;
