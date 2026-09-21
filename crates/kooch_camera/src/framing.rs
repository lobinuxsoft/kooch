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
    /// Seconds to close most of the gap while the target is in the soft zone. Zero is rigid.
    #[reflect(range = TIME_RANGE)]
    pub soft_time: f32,
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
            soft_time: 0.3,
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
    /// Where the rig follows this step: `tracked` moved just enough to bring `target` back towards
    /// the dead zone, measured on the screen of a camera turned `rotation` at `depth`. Depth itself
    /// has no zone and is followed at once.
    pub fn follow(
        &self,
        tracked: Vec3,
        target: Vec3,
        rotation: Quat,
        depth: f32,
        lens: Lens,
        dt: f32,
    ) -> Vec3 {
        let (right, up, forward) = (rotation * Vec3::X, rotation * Vec3::Y, rotation * -Vec3::Z);
        let span = lens.span(depth);
        let offset = target - tracked;
        let on_screen = Vec2::new(offset.dot(right) / span.x, offset.dot(up) / span.y);
        let soft = self.soft_zone.max(self.dead_zone);
        let moved = Vec2::new(
            self.correction(on_screen.x, self.dead_zone.x, soft.x, dt),
            self.correction(on_screen.y, self.dead_zone.y, soft.y, dt),
        ) * span;
        tracked + right * moved.x + up * moved.y + forward * offset.dot(forward)
    }

    /// The point to look at so `tracked` lands on [`screen`](Self::screen).
    pub fn aim(&self, tracked: Vec3, rotation: Quat, depth: f32, lens: Lens) -> Vec3 {
        let shift = self.screen * lens.span(depth);
        tracked - rotation * Vec3::X * shift.x - rotation * Vec3::Y * shift.y
    }

    /// One screen axis: nothing inside the dead zone, the excess past the soft zone at once, and
    /// the part between eased over `soft_time`.
    fn correction(&self, offset: f32, dead: f32, soft: f32, dt: f32) -> f32 {
        let outside = offset.abs() - dead.max(0.0) * 0.5;
        if outside <= 0.0 {
            return 0.0;
        }
        let hard = (offset.abs() - soft * 0.5).max(0.0);
        let alpha = match self.soft_time > 0.0 && dt > 0.0 {
            true => 1.0 - (-dt / self.soft_time).exp(),
            false => 1.0,
        };
        offset.signum() * (hard + (outside - hard) * alpha)
    }
}

/// Every framed vcam's tracked point, carried between steps. Rebuilt from the vcams seen each step,
/// so a despawned one leaves nothing behind.
#[derive(Debug, Clone, Default)]
pub struct Tracked {
    points: HashMap<Entity, Vec3>,
}

impl Tracked {
    /// Where this vcam's rig was following, or `None` on its first framed step.
    pub fn of(&self, entity: Entity) -> Option<Vec3> {
        self.points.get(&entity).copied()
    }

    pub fn set(&mut self, entity: Entity, point: Vec3) {
        self.points.insert(entity, point);
    }
}

#[cfg(test)]
mod tests;
