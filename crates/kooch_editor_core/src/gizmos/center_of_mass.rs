//! [`CenterOfMassVisualizer`] — draws the centre of mass an author placed.
//!
//! # What this shows, and what it does not
//!
//! The **authored** centre of mass: `PhysicsBody.center_of_mass`, when
//! `center_of_mass_enabled` is on. That is a component, so it mirrors from
//! a remote project like any other and can be drawn here.
//!
//! It is *not* where the solver ended up putting it. Those agree whenever
//! the override is on — the backend copies the authored point into the
//! body's mass properties — and the interesting case is the other one: a
//! compound body with the override **off**, whose centre of mass the solver
//! computes from the shapes and which surprised the author of #618. That
//! number lives in the solver, and the solver is in the project's process,
//! so drawing it needs the overlay to cross the wire (#634).
//!
//! So this answers "where did I put it", and #634 answers "where is it".
//! Both are worth having and only one of them is a component.

use glam::Vec3;

use kooch_ecs::hierarchy::GlobalTransform;
use kooch_gizmos::{Gizmos, Visualizer};
use kooch_physics::components::PhysicsBody;

/// Amber, matching the Inspector's physics warnings — this is the same
/// family of "the physics is not where you assume" information.
const MARKER: Vec3 = Vec3::new(0.95, 0.7, 0.15);

/// Radius of the marker ball, in world units before scaling.
const RADIUS: f32 = 0.08;

/// Length of the crosshair arms, longer than the ball so the point stays
/// findable when the ball is behind geometry.
const ARM: f32 = 0.25;

#[derive(Default)]
pub(crate) struct CenterOfMassVisualizer;

impl Visualizer<PhysicsBody> for CenterOfMassVisualizer {
    fn draw(&self, body: &PhysicsBody, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>) {
        // Nothing authored means nothing to say: with the override off the
        // solver derives the point, and drawing a guess at it would be the
        // kind of gizmo that lies. See the module docs.
        let Some(local) = body.explicit_center_of_mass() else {
            return;
        };

        let (scale, rotation, translation) = transform.matrix.to_scale_rotation_translation();
        // The same composition the solver applies: the offset is in the
        // entity's local space, so it scales and rotates with the entity.
        let world = translation + rotation * (local * scale);
        let basis = glam::Mat3::from_quat(rotation);

        // A ball for "here", and arms so it can be found when the ball is
        // inside the mesh it belongs to — which, being a centre of mass, it
        // usually is.
        gizmos.wire_sphere(world, basis, RADIUS, MARKER);
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            let arm = basis * (axis * ARM);
            gizmos.line(world - arm, world + arm, MARKER);
        }
    }
}

#[cfg(test)]
mod tests;
