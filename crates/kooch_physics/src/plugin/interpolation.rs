//! Drawing a body between two steps (#1415).
//!
//! 🔴 The solver answers 60 times a second and the frame is drawn at whatever the display does. The
//! writeback copied each pose straight onto `Transform`, so two frames in three showed the previous
//! step's — a body micro-stepping across the screen. It was always true and was hidden while the
//! camera rode the same fixed clock: rig and target stepped together, so their relative motion was
//! constant and the eye had nothing to compare against. #1413 put the camera on the frame, and the
//! stepping came out from under it.
//!
//! `Transform` carries the DRAWN pose, as Unity's `Rigidbody.interpolation` does; the simulated one
//! stays in the solver, reachable through `PhysicsWorld`. That is safe here for a concrete reason:
//! `push_authored_poses` skips dynamic bodies while playing, so nothing reads this back into the
//! solver and calls it an authored move.

use std::collections::HashMap;

use glam::{Quat, Vec3};
use kooch_core::resource::Resources;
use kooch_core::run_state::Playing;
use kooch_core::time::Time;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::transform::Transform;

/// 🔴 Past this, a body did not move — it was PUT somewhere. Sliding across a teleport is a body
/// smeared through a wall for a frame, and an authored push is a cut by definition. Metres, and
/// generous: a body doing 60 m/s covers one metre in a 60 Hz step, so only a jump no motion could
/// produce lands above it.
const TELEPORT: f32 = 2.0;

/// The last two steps' poses per body, so a frame can be drawn between them.
#[derive(Debug, Default)]
pub struct StepPoses {
    poses: HashMap<Entity, Step>,
}

#[derive(Debug, Clone, Copy)]
struct Step {
    from: (Vec3, Quat),
    to: (Vec3, Quat),
}

impl StepPoses {
    /// Records what the step just produced, keeping what it replaced.
    pub fn stepped(&mut self, entity: Entity, position: Vec3, rotation: Quat) {
        match self.poses.get_mut(&entity) {
            Some(step) => {
                step.from = step.to;
                step.to = (position, rotation);
            }
            // A body's first step has nothing to come from, so it is drawn where it is.
            None => {
                self.poses.insert(
                    entity,
                    Step {
                        from: (position, rotation),
                        to: (position, rotation),
                    },
                );
            }
        }
    }

    /// Drops every body not in `alive`, so a despawned entity's pose is not carried into whatever
    /// reuses its index.
    pub fn retain(&mut self, alive: &[Entity]) {
        self.poses.retain(|entity, _| alive.contains(entity));
    }

    /// Where `entity` is drawn at `alpha` through the step.
    fn at(&self, entity: Entity, alpha: f32) -> Option<(Vec3, Quat)> {
        let step = self.poses.get(&entity)?;
        if step.from.0.distance_squared(step.to.0) > TELEPORT * TELEPORT {
            return Some(step.to);
        }
        Some((
            step.from.0.lerp(step.to.0, alpha),
            // The short way: `q` and `-q` are one rotation, and without matching them a body can
            // spin the long way round inside a single step.
            short_slerp(step.from.1, step.to.1, alpha),
        ))
    }
}

fn short_slerp(from: Quat, to: Quat, t: f32) -> Quat {
    let to = if from.dot(to) < 0.0 { -to } else { to };
    from.slerp(to, t).normalize()
}

/// Draws every simulated body between the last two steps.
///
/// 🔴 Once per frame, before transform propagation publishes what the renderer reads, and before
/// the camera rig looks at its target — a camera following an un-interpolated target is the stutter
/// this exists to remove.
pub fn interpolate_bodies(resources: &mut Resources) {
    if !Playing::is_playing(resources) {
        return;
    }
    let Some(alpha) = resources.get::<Time>().map(|time| time.render_alpha()) else {
        return;
    };
    // 🔴 `!(x >= 0.0)` rather than a comparison that lets NaN through: an accumulator divided by a
    // zero step is NaN, and a NaN here would put every body nowhere at once, silently.
    if !(alpha >= 0.0) || !(alpha <= 1.0) {
        return;
    }
    let Some(poses) = resources.remove::<StepPoses>() else {
        return;
    };
    let mut moved: Vec<Entity> = Vec::new();
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(transforms) = registry.get_cpu_mut::<Transform>()
    {
        for (&entity, _) in poses.poses.iter() {
            let Some((position, rotation)) = poses.at(entity, alpha) else {
                continue;
            };
            if let Some(transform) = transforms.get_mut(entity) {
                transform.position = position;
                transform.rotation = rotation;
                moved.push(entity);
            }
        }
    }
    resources.insert(poses);
    // 🔴 Published before anything reads it. The camera rig follows its target through
    // `GlobalTransform`, so without this it chased the pose from before the step while the body was
    // DRAWN interpolated — the body stopped stepping and the camera started (#1423).
    //
    // Only these subtrees. A full pass rebuilds two maps over every entity in the scene to
    // republish the handful that moved.
    kooch_ecs::hierarchy::propagate_subtrees(resources, &moved);
}

#[cfg(test)]
mod tests;
