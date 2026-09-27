//! [`CameraFraming`] — where on screen a vcam holds its target, and how far the target may wander
//! before the camera answers: Cinemachine's composer dead and soft zones (#1252).
//!
//! 🔴 Framing moves the camera; it does **not** turn it (#1329). In a third-person rig the rotation
//! belongs to the player — it is how you look around — so a framing that owned it fought every
//! stick input and never settled. Cinemachine's composer owns the rotation because its body is
//! placed independently; for an orbiting rig Cinemachine uses `ThirdPersonFollow` and no composer
//! at all. Phantom Camera's Framed mode is the right shape for ours.
//!
//! One quantity is eased here — where the rig follows — and the zones are read in **camera** space
//! and answered in camera space. Phantom tests on screen and responds along world axes, which is
//! fine until "up" is the face of a cube planet.
//!
//! An axis inside the dead zone is anchored to where the rig already is, **before** the ease rather
//! than corrected after it. That is the whole of "the camera holds still", and shoving the ease's
//! own answer afterwards is what used to make it jump.

use std::collections::HashMap;

use glam::{Vec2, Vec3};
use kooch_ecs::Reflect;
use kooch_ecs::component::Component;
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::FieldRange;
use kooch_ecs::tween::Chase;

use crate::frame::CameraFrame;

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
    /// Public because a gizmo needs it: putting a point on screen is exactly this, and a second
    /// copy of the arithmetic in the editor is a second place for it to drift.
    pub fn span(&self, depth: f32) -> Vec2 {
        Vec2::new(self.half_width, self.half_height) * 2.0 * depth.max(0.01)
    }
}

impl CameraFraming {
    /// Where the rig should follow, so the target lands where it belongs on screen.
    ///
    /// `wanted` is where the rig would put the camera with no framing at all. The answer is that
    /// point moved across the screen's own axes — never along its forward, since how far away the
    /// camera sits is the rig's business and not the frame's.
    /// The Frame stage: moves the camera sideways so the target lands where it is held. Never
    /// along the forward — how far away the camera sits is the body's.
    pub fn frame(&self, state: &mut Framed, frame: &mut CameraFrame, dt: f32) {
        let (right, above, forward) = frame.axes();
        let placed = |slack: Vec2| frame.free + right * slack.x + above * slack.y;

        // 🔴 Read off where the camera actually is against what the body asked for, never carried
        // in a field of its own: a slack remembered rather than measured is a second opinion about
        // where the camera stands, which is the shape of every bug this component has had. `free`
        // is the body's answer, so a wall that moved the camera is not read as slack (#1330).
        let behind = frame.previous - frame.free;
        let slack = Vec2::new(behind.dot(right), behind.dot(above));

        let target = frame.target;
        let depth = (target - placed(slack)).dot(forward);
        if depth <= 0.0 {
            // Behind the camera: there is no screen to frame it on.
            state.reset(target);
            frame.displace(frame.free);
            return;
        }
        let span = frame.lens.span(depth);

        // Where the target sits, as a fraction of the screen from where it belongs. `frame.screen`
        // already carries the lead, so one thing decides where the character sits (#1330).
        let offset = target - placed(slack);
        let held = self.screen + frame.screen;
        let at = Vec2::new(offset.dot(right) / span.x, offset.dot(above) / span.y) - held;
        let soft = self.soft_zone.max(self.dead_zone);

        // What the frame owes: nothing inside the dead zone, the excess outside it. Moving the
        // camera by `d` along an axis moves the target by `-d` on screen, so the slack owed is the
        // excess itself, in metres.
        let owed = past(at, self.dead_zone);
        let still = state.last.abs_diff_eq(target, STILL);
        let share = match still {
            true => Vec2::ONE,
            false => Vec2::new(
                ramp(at.x, self.dead_zone.x, soft.x),
                ramp(at.y, self.dead_zone.y, soft.y),
            ),
        };
        let goal = slack + owed * share * span;

        // One quantity, eased once: the slack. The body is not damped when a frame is present —
        // two eases in series on one position is what made this fight itself (#1329).
        let eased = state.chase.step(slack, goal, dt, self.soft_duration);
        state.slack = eased;
        state.last = target;
        frame.displace(placed(eased));
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
    /// How far the frame holds the camera off what the rig asked for, along the screen's own axes,
    /// in metres.
    slack: Vec2,
    /// The ease closing what the dead zone does not forgive.
    chase: Chase<Vec2>,
    last: Vec3,
}

impl Framed {
    /// How far the frame is holding the camera off the body's answer, along the screen's axes.
    /// What a gizmo draws: the zones say what was asked for, this says what the frame is doing.
    pub fn slack(&self) -> Vec2 {
        self.slack
    }

    /// Framed on a target that has not moved yet.
    pub fn at(target: Vec3) -> Self {
        Self {
            slack: Vec2::ZERO,
            chase: Chase::at(Vec2::ZERO),
            last: target,
        }
    }

    /// Back on the rig's own answer, with nothing owed.
    fn reset(&mut self, target: Vec3) {
        self.slack = Vec2::ZERO;
        self.chase = Chase::at(Vec2::ZERO);
        self.last = target;
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
