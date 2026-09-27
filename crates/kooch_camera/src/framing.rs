//! [`CameraFraming`] — where on screen a vcam holds its target, and how far the target may wander
//! before the camera answers: Cinemachine's composer dead and soft zones (#1252).
//!
//! 🔴 Framing **aims**; it does not move the camera (#1323). The rig places the camera and damps it
//! like any other, and this turns it so the target lands where it should. Where a target sits on
//! screen is the product of a position and a rotation, so an owner for one of them is an owner for
//! nothing: this used to ease the position while the vcam damped the rotation underneath it, with
//! two durations, and the two fought — the shake nobody could place.
//!
//! One quantity is eased here, the rotation, and the zones are limits on its **goal**. Nothing
//! reads back or shoves the ease's own answer, which is what makes the result animatable: the rig
//! hands out one pose, and a timeline that wants the camera can take it whole.

use std::collections::HashMap;

use glam::{Quat, Vec2, Vec3};
use kooch_ecs::Reflect;
use kooch_ecs::component::Component;
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::FieldRange;
use kooch_ecs::tween::Chase;

/// Frames the target of the vcam it sits on. Beside a [`VirtualCamera`]; replaces its position
/// damping, since the soft zone is the easing.
///
/// [`VirtualCamera`]: crate::VirtualCamera
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Camera")]
pub struct CameraFraming {
    /// Off aims at the target itself, as without this component. The rig follows it either way.
    pub enabled: bool,
    /// Where the target sits on screen: `0` is the centre, `±0.5` the edges, +Y up.
    #[reflect(range = SCREEN_RANGE)]
    pub screen: Vec2,
    /// Width and height, as fractions of the screen, the target moves inside with the camera not
    /// turning at all.
    #[reflect(range = ZONE_RANGE)]
    pub dead_zone: Vec2,
    /// Width and height over which the camera comes up to speed: none of the correction on the dead
    /// zone's edge, all of it here. Never smaller than the dead zone.
    ///
    /// 🔴 A band, not a wall. Without it the camera goes from not turning to turning at full rate
    /// between two frames, and that step is what reads as a shake; a wall that shoved the camera
    /// back measured three times worse than the rig on its own.
    #[reflect(range = ZONE_RANGE)]
    pub soft_zone: Vec2,
    /// Seconds the camera takes to bring the target back to the dead zone's edge **once it stops** —
    /// exactly, a tween that restarts while the target keeps moving. Zero is rigid.
    #[reflect(range = TIME_RANGE, alias = "soft_time")]
    pub soft_duration: f32,
}

/// How far a target or a camera may drift and still count as standing still, in metres.
const STILL: f32 = 1e-5;

const SCREEN_RANGE: FieldRange = FieldRange {
    min: -0.5,
    max: 0.5,
    step: 0.01,
};

const ZONE_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 2.0,
    step: 0.01,
};

const TIME_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 3.0,
    step: 0.01,
};

impl Default for CameraFraming {
    fn default() -> Self {
        Self {
            enabled: true,
            screen: Vec2::ZERO,
            dead_zone: Vec2::new(0.1, 0.1),
            soft_zone: Vec2::new(0.6, 0.6),
            soft_duration: 0.5,
        }
    }
}

impl Component for CameraFraming {}

/// How much of the world a view shows at one metre: half its height and half its width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lens {
    pub half_height: f32,
    pub half_width: f32,
}

impl Lens {
    /// From a vertical field of view in degrees and a width over height.
    pub fn new(fov: f32, aspect: f32) -> Self {
        let half_height = (fov.clamp(1.0, 179.0).to_radians() * 0.5).tan();
        Self {
            half_height,
            half_width: half_height * aspect.max(0.01),
        }
    }

    /// The screen's size in metres at `depth`.
    fn span(&self, depth: f32) -> Vec2 {
        Vec2::new(self.half_width, self.half_height) * 2.0 * depth.max(0.01)
    }
}

impl CameraFraming {
    /// The rotation that frames `target` from `eye`, eased from `current`.
    ///
    /// Inside the dead zone nothing is owed and the camera holds still. Outside it, the goal is the
    /// rotation that puts the target back on the dead zone's edge, and [`soft_duration`] is how
    /// long that takes once the target stops — exactly, since the tween restarts while the goal
    /// keeps moving.
    ///
    /// [`soft_duration`]: Self::soft_duration
    pub fn aim(
        &self,
        state: &mut Framed,
        eye: Vec3,
        current: Quat,
        target: Vec3,
        up: Vec3,
        reference: Vec3,
        lens: Lens,
        dt: f32,
    ) -> Quat {
        let Some(offset) = (target - eye).try_normalize().map(|_| target - eye) else {
            return current;
        };
        let forward = current * -Vec3::Z;
        let depth = offset.dot(forward);
        if depth <= 0.0 {
            // Behind the camera: there is no screen to frame it on, and the rig is about to swing
            // round anyway.
            return current;
        }

        let (right, above) = (current * Vec3::X, current * Vec3::Y);
        let span = lens.span(depth);
        // Where the target is now, as a fraction of the screen from its centre.
        let seen = Vec2::new(offset.dot(right) / span.x, offset.dot(above) / span.y);
        // And how far that is from where it belongs.
        let at = seen - self.screen;
        let soft = self.soft_zone.max(self.dead_zone);

        // What the camera owes: nothing inside the dead zone, the excess outside it. Owing nothing
        // means holding still — the rotation it already has, not one rebuilt from a screen
        // position, which would drift by the tangent plane's own error every step.
        let owed = past(at, self.dead_zone);
        let held = state.rotation.unwrap_or(current);
        if owed == Vec2::ZERO {
            state.chase = Chase::at(held);
            state.goal = None;
            state.was_still = false;
            state.last = target;
            state.eye = eye;
            return held;
        }

        // 🔴 The soft zone limits where the ease MAY be, and it does so by telling the ease where
        // it stands before it steps — Phantom Camera's trick of anchoring an axis ahead of the
        // damper rather than correcting after it, in camera space so an arbitrary up survives it.
        // Shoving its answer afterwards is what made the camera jump at speed (#1323).
        // 🔴 The soft zone RAMPS the correction in: none at the dead zone's edge, all of it at the
        // soft one. Switching the ease on at the dead edge is a step in the frame's speed — the
        // camera goes from not turning at all to turning at full rate between two frames — and a
        // dead zone without a band around it can only do that. This is what the soft zone is for,
        // and the old code had the idea in the wrong place: it carried the target's velocity into
        // the point instead of scaling the goal (#1323).
        //
        // 🔴 Only while the target is moving. Ramped to nothing at the dead zone's edge, the camera
        // would approach it and never arrive, and `soft_duration` promises a time. Once the target
        // stops, the whole correction applies and the tween lands on the edge exactly when it says.
        // The two phases are the two things being asked for, and neither can be had alone.
        let still = state.last.abs_diff_eq(target, STILL) && state.eye.abs_diff_eq(eye, STILL);
        let share = match still {
            true => Vec2::ONE,
            false => Vec2::new(
                ramp(at.x, self.dead_zone.x, soft.x),
                ramp(at.y, self.dead_zone.y, soft.y),
            ),
        };
        let from = held;

        // The goal: the target back on the dead zone's edge.
        //
        // 🔴 Held while nothing moves. Measured from the eased rotation it would chase itself — a
        // goal a hair different every frame, so the tween restarts every frame and never finishes.
        // The promise is "this long after the target stops", and a goal that never settles cannot
        // keep it.
        // Held only once it has been still for a step: the frame the target stops on is the frame
        // the full correction is computed, and holding before that would freeze the ramped goal.
        let goal = match (still && state.was_still, state.goal) {
            (true, Some(goal)) => goal,
            _ => self.turned(
                eye,
                current,
                target,
                seen - owed * share,
                lens,
                up,
                reference,
            ),
        };
        state.goal = Some(goal);
        state.was_still = still;
        state.eye = eye;
        let eased = state.chase.step(from, goal, dt, self.soft_duration);
        state.rotation = Some(eased);
        state.last = target;
        eased
    }

    /// The rotation that lands `target` at the screen position `seen`, in fractions of the screen
    /// from its centre. Looking straight at a point puts it in the middle, so the point to look at
    /// is the target shifted back by where it should appear.
    ///
    /// 🔴 Solved rather than computed: the shift is measured in the screen's plane, and turning the
    /// camera moves that plane. Three passes take the residual below a thousandth of the screen,
    /// which is the difference between landing ON the dead zone's edge and near it.
    #[allow(clippy::too_many_arguments)]
    fn turned(
        &self,
        eye: Vec3,
        current: Quat,
        target: Vec3,
        seen: Vec2,
        lens: Lens,
        up: Vec3,
        reference: Vec3,
    ) -> Quat {
        let offset = target - eye;
        let mut rotation = current;
        for _ in 0..3 {
            let depth = offset.dot(rotation * -Vec3::Z);
            if depth <= 0.0 {
                return rotation;
            }
            let span = lens.span(depth);
            let (right, above) = (rotation * Vec3::X, rotation * Vec3::Y);
            let aim = target - right * (seen.x * span.x) - above * (seen.y * span.y);
            rotation = crate::virtual_camera::look_at(eye, aim, up, reference);
        }
        rotation
    }
}

/// How much of the correction applies at `at`: none on the dead zone's edge, all of it on the soft
/// one, and all of it beyond. A band of zero width is a step, which is what a soft zone the same
/// size as the dead one asks for.
fn ramp(at: f32, dead: f32, soft: f32) -> f32 {
    let band = (soft - dead).max(0.0) * 0.5;
    match band > 0.0 {
        true => ((at.abs() - dead.max(0.0) * 0.5) / band).clamp(0.0, 1.0),
        false => 1.0,
    }
}

/// How far past a zone of `size` a screen offset sits, per axis, in screen fractions.
fn past(at: Vec2, size: Vec2) -> Vec2 {
    let beyond = |at: f32, size: f32| at.signum() * (at.abs() - size.max(0.0) * 0.5).max(0.0);
    Vec2::new(beyond(at.x, size.x), beyond(at.y, size.y))
}

/// One vcam's framing in flight: where its rig follows, the tween closing the excess, and where the
/// target was last step.
#[derive(Debug, Clone, Copy)]
pub struct Framed {
    /// The rotation the ease last answered. `None` until it has answered once.
    rotation: Option<Quat>,
    /// The ease closing what the dead zone does not forgive.
    chase: Chase<Quat>,
    /// Where the ease is heading, held while neither the target nor the camera moves.
    goal: Option<Quat>,
    /// Whether nothing moved last step, so the goal is held from the second still step on.
    was_still: bool,
    last: Vec3,
    eye: Vec3,
}

impl Framed {
    /// The rotation it last answered, before anything else touched the pose.
    pub fn rotation(&self) -> Option<Quat> {
        self.rotation
    }

    /// Framed on a target that has not moved yet.
    pub fn at(target: Vec3) -> Self {
        Self {
            rotation: None,
            chase: Chase::at(Quat::IDENTITY),
            goal: None,
            was_still: false,
            last: target,
            eye: Vec3::ZERO,
        }
    }
}

/// Every framed vcam's tracked point, carried between steps. Rebuilt from the vcams seen each step,
/// so a despawned one leaves nothing behind.
#[derive(Debug, Clone, Default)]
pub struct Tracked {
    points: HashMap<Entity, Framed>,
}

impl Tracked {
    /// This vcam's framing, or `None` on its first framed step.
    pub fn of(&self, entity: Entity) -> Option<Framed> {
        self.points.get(&entity).copied()
    }

    pub fn set(&mut self, entity: Entity, framed: Framed) {
        self.points.insert(entity, framed);
    }
}

#[cfg(test)]
mod tests;
