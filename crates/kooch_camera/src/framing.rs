//! [`CameraFraming`] — where on screen a vcam holds its target, and how far the target may wander
//! before the camera answers: Cinemachine's composer dead and soft zones (#1252).
//!
//! The rig follows a tracked point instead of the target. Inside the dead zone the point stays put,
//! in the soft zone it eases after the target, and past the soft zone it is dragged along.

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
    /// Off follows the target itself, as without this component.
    pub enabled: bool,
    /// Where the target sits on screen: `0` is the centre, `±0.5` the edges, +Y up.
    #[reflect(range = SCREEN_RANGE)]
    pub screen: Vec2,
    /// Width and height, as fractions of the screen, the target moves inside with the camera still.
    #[reflect(range = ZONE_RANGE)]
    pub dead_zone: Vec2,
    /// Width and height past which the camera keeps the target at the edge. Between it and the dead
    /// zone the camera eases back; never smaller than the dead zone.
    #[reflect(range = ZONE_RANGE)]
    pub soft_zone: Vec2,
    /// Seconds the camera takes to bring the target back to the dead zone's edge once it stops —
    /// exactly, a tween that restarts while the target keeps moving. Past the soft zone the camera
    /// does not wait. Zero is rigid.
    #[reflect(range = TIME_RANGE, alias = "soft_time")]
    pub soft_duration: f32,
}

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
    /// Where the rig follows this step. Two terms, both continuous: the target's own motion, taken
    /// on as the target crosses the soft zone, and a tween closing what is left of the excess.
    ///
    /// 🔴 Without the first term the camera never reaches the target's speed, hits the soft zone's
    /// wall and doubles its speed in one step — the stutter of #1288. Weighted by how deep into the
    /// soft zone the target is, it arrives at the wall already travelling alongside it.
    pub fn follow(
        &self,
        state: &mut Framed,
        target: Vec3,
        rotation: Quat,
        depth: f32,
        lens: Lens,
        dt: f32,
    ) -> Vec3 {
        let (right, up, forward) = (rotation * Vec3::X, rotation * Vec3::Y, rotation * -Vec3::Z);
        let span = lens.span(depth);
        let tracked = state.point;
        let soft = self.soft_zone.max(self.dead_zone);

        // Where the target sits, as a fraction of the screen from the tracked point.
        let on_screen = |point: Vec3| {
            let offset = target - point;
            Vec2::new(offset.dot(right) / span.x, offset.dot(up) / span.y)
        };
        // How far past a zone of `size` it sits, in metres on screen.
        let past = |point: Vec3, size: Vec2| {
            let at = on_screen(point);
            let beyond =
                |at: f32, size: f32| at.signum() * (at.abs() - size.max(0.0) * 0.5).max(0.0);
            let excess = Vec2::new(beyond(at.x, size.x), beyond(at.y, size.y)) * span;
            right * excess.x + up * excess.y
        };
        // 0 on the dead zone's edge, 1 on the soft zone's: how much of the target's motion the
        // camera takes on, per screen axis.
        let carried = |at: f32, dead: f32, soft: f32| {
            let band = (soft - dead).max(0.0) * 0.5;
            match band > 0.0 {
                true => ((at.abs() - dead.max(0.0) * 0.5) / band).clamp(0.0, 1.0),
                false => 1.0,
            }
        };

        let at = on_screen(tracked);
        let share = Vec2::new(
            carried(at.x, self.dead_zone.x, soft.x),
            carried(at.y, self.dead_zone.y, soft.y),
        );
        let moved = target - state.last;
        state.last = target;
        // A step wider than the soft band is a teleport, not a speed: the wall below places the
        // point for it rather than the camera flying there.
        let band = ((soft - self.dead_zone) * 0.5 * span).max(Vec2::ZERO);
        let carry = |along: Vec3, share: f32, band: f32| {
            along * moved.dot(along).clamp(-band, band) * share
        };
        let led = tracked + carry(right, share.x, band.x) + carry(up, share.y, band.y);

        // The wall, if the first term was not enough: it moves where the tween STARTS, never what
        // it returned — a point shoved afterwards is one the tween pulls back next step.
        let from = led + past(led, soft);
        let goal = from + past(from, self.dead_zone);
        let point = chase_step(&mut state.chase, from, goal, dt, self.soft_duration);
        state.point = point + forward * (target - point).dot(forward);
        state.point
    }

    /// The point to look at so `tracked` lands on [`screen`](Self::screen).
    pub fn aim(&self, tracked: Vec3, rotation: Quat, depth: f32, lens: Lens) -> Vec3 {
        let shift = self.screen * lens.span(depth);
        tracked - rotation * Vec3::X * shift.x - rotation * Vec3::Y * shift.y
    }
}

/// One vcam's framing in flight: where its rig follows, the tween closing the excess, and where the
/// target was last step.
#[derive(Debug, Clone, Copy)]
pub struct Framed {
    point: Vec3,
    chase: Chase<Vec3>,
    last: Vec3,
}

impl Framed {
    /// Where the rig is following.
    pub fn point(&self) -> Vec3 {
        self.point
    }

    /// Framed on a target that has not moved yet.
    pub fn at(target: Vec3) -> Self {
        Self {
            point: target,
            chase: Chase::at(target),
            last: target,
        }
    }
}

/// `Chase::step`, named so the framing reads as one expression.
fn chase_step(chase: &mut Chase<Vec3>, from: Vec3, goal: Vec3, dt: f32, duration: f32) -> Vec3 {
    chase.step(from, goal, dt, duration)
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
