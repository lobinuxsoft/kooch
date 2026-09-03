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
//! # The mesh-derived shapes
//!
//! A hull's outline is its own point cloud, which lives in
//! [`ColliderMeshCache`] — so those go through
//! [`Visualizer::draw_with`], the one path that gets `Resources`.
//!
//! Two of them still draw nothing, and it is not an omission. A triangle
//! mesh and an underived decomposition *are* the render mesh, edge for
//! edge: outlining them draws a second copy of what is already on screen,
//! at a hundred thousand lines a frame. The outlines worth having are the
//! ones that differ from what you can see — the hull, and the pieces.
//!
//! [`ColliderMeshCache`]: kooch_physics::ColliderMeshCache

use glam::{Mat3, Vec3};

use kooch_core::resource::Resources;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_gizmos::{Gizmos, Visualizer};
use kooch_physics::backend::{ColliderMeshCache, CollisionShape, ConvexPart};
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
    fn draw_with(
        &self,
        collider: &Collider,
        transform: &GlobalTransform,
        _entity: Entity,
        resources: &Resources,
        gizmos: &mut Gizmos<'_>,
    ) {
        if is_mesh_derived(collider.shape) {
            let (scale, rotation, translation) = transform.matrix.to_scale_rotation_translation();
            let centre = translation + rotation * (collider.center * scale.abs());
            draw_mesh_shape(
                collider,
                resources,
                Mat3::from_quat(rotation),
                centre,
                scale,
                gizmos,
            );
            return;
        }
        self.draw(collider, transform, gizmos);
    }

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

        // Handled by `draw_with`, which can reach the points. A sphere
        // here would be a shape the solver is not using.
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

/// The mesh-derived outlines, drawn from the cache the solver reads.
///
/// Same geometry the solver was handed, scale folded in the same way —
/// `ShapeSpec::resolve` is the single place that decides both, so the
/// outline cannot drift from the collider the way a second derivation
/// would.
fn draw_mesh_shape(
    collider: &Collider,
    resources: &Resources,
    basis: Mat3,
    translation: Vec3,
    scale: Vec3,
    gizmos: &mut Gizmos<'_>,
) {
    let meshes = resources.get::<ColliderMeshCache>();
    let Some(shape) = collider.shape_spec(meshes).resolve(meshes) else {
        // The mesh has not arrived. The body has not been built either,
        // so an outline would be the only thing in the scene claiming
        // there is a collider here.
        return;
    };

    match shape.scaled(scale) {
        CollisionShape::ConvexHull { part } => {
            draw_part(&part, basis, translation, gizmos);
        }
        CollisionShape::Compound { parts } => {
            for part in &parts {
                draw_part(part, basis, translation, gizmos);
            }
        }
        // A triangle mesh is the render mesh, edge for edge. Drawing it
        // is a second copy of what is on screen, at a hundred thousand
        // lines a frame.
        _ => {}
    }
}

/// One convex piece, in the entity's space.
///
/// Only when the faces are known: deriving them here would run qhull
/// inside a draw call, once per selected entity per frame.
fn draw_part(part: &ConvexPart, basis: Mat3, translation: Vec3, gizmos: &mut Gizmos<'_>) {
    if !part.is_hulled() || part.faces.len() > MAX_OUTLINE_FACES {
        return;
    }
    let points: Vec<Vec3> = part
        .points
        .iter()
        .map(|point| translation + basis * *point)
        .collect();
    gizmos.wire_triangles(&points, &part.faces, SOLID);
}

/// Faces above which an outline stops being a drawing and starts being a
/// wall of lines.
///
/// A hull is a few hundred; a decomposition is a few hundred per piece.
/// Anything past this is not a shape anyone reads off the screen.
const MAX_OUTLINE_FACES: usize = 4096;

/// How far the half-space patch reaches from the shape's centre.
///
/// An infinite plane cannot be drawn; this is the suggestion of one, big
/// enough to read as ground and small enough not to swallow the scene.
const PLANE_PATCH: f32 = 5.0;

#[cfg(test)]
mod tests;
