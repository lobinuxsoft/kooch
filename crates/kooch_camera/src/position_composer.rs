//! [`PositionComposer`] — the Body that holds the target at a screen point by **moving** the camera:
//! Cinemachine's `CinemachinePositionComposer` (#1369).
//!
//! 🔴 The other half of the pair. `RotationComposer` turns the camera where it stands; this one
//! moves it where it looks, and they are two components because they are two jobs in two slots.
//! What #1329 built and #1361 took out of the Aim was this, in the wrong slot all along.
//!
//! Depth first — the camera slides along its own forward until the target is `camera_distance`
//! away — then the screen plane. Both in the **camera's** axes, whose up is the vcam's, so on the
//! side of a planet it slides along the local horizon and not world X.

use glam::{Vec2, Vec3};
use kooch_ecs::Reflect;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::FieldRange;

use crate::frame::CameraFrame;
use crate::framing::{SCREEN_RANGE, TIME_RANGE, ZONE_RANGE, past, ramp};

/// Positions the vcam it sits on so its target lands where it belongs on screen. Read only by a vcam
/// whose `follow` is `Position Composer`; the vcam's own `damping_value` does not also run.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Camera")]
pub struct PositionComposer {
    /// How far the target sits from the camera's screen plane.
    #[reflect(range = DISTANCE_RANGE)]
    pub camera_distance: f32,
    /// How much that distance may vary before the camera answers it.
    #[reflect(range = DISTANCE_RANGE)]
    pub dead_zone_depth: f32,
    /// Where the target sits on screen: `0` the centre, `±0.5` the edges, `+y` up.
    #[reflect(range = SCREEN_RANGE)]
    pub screen_position: Vec2,
    /// Width and height, as fractions of the screen, the target moves inside with the camera not
    /// moving at all.
    #[reflect(range = ZONE_RANGE)]
    pub dead_zone: Vec2,
    /// Width and height over which the camera comes up to speed. Cinemachine has hard limits here
    /// instead; a correction that snaps on at the dead zone's edge reads as a step (#1329).
    #[reflect(range = ZONE_RANGE)]
    pub soft_zone: Vec2,
    /// Seconds to close the gap, per **camera** axis: right, up, forward.
    ///
    /// Not Cinemachine's `Damping`, which is a per-axis "how aggressively". This is the duration
    /// `VirtualCamera::damping_value` already means, and borrowing the other name would promise a
    /// number it does not deliver (#1367).
    #[reflect(range = TIME_RANGE)]
    pub damping_value: Vec3,
    /// Whether taking over centres the target at once, rather than easing it in from wherever the
    /// camera was left.
    pub center_on_activate: bool,
}

/// Never nose-first through the target: a zero distance has no screen plane to compose on.
const NEAREST: f32 = 0.01;

const DISTANCE_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 100.0,
    step: 0.1,
};

impl Default for PositionComposer {
    fn default() -> Self {
        Self {
            camera_distance: 10.0,
            dead_zone_depth: 0.0,
            screen_position: Vec2::ZERO,
            dead_zone: Vec2::splat(0.1),
            soft_zone: Vec2::splat(0.8),
            damping_value: Vec3::splat(0.5),
            center_on_activate: true,
        }
    }
}

impl Component for PositionComposer {}

/// The composer on `vcam`, if it has one. The only place that question is asked.
pub fn of(registry: &ComponentRegistry, vcam: Entity) -> Option<PositionComposer> {
    registry.get_cpu::<PositionComposer>()?.get(vcam).copied()
}

impl PositionComposer {
    /// Where the camera should stand so the target lands where it belongs, starting from where the
    /// camera already is — this **is** the body, so there is no other answer to measure against.
    pub fn placed(&self, frame: &CameraFrame, fresh: bool, dt: f32) -> Vec3 {
        let (right, above, forward) = frame.axes();
        let from = frame.previous;
        let to = frame.target - from;
        let at = Vec3::new(to.dot(right), to.dot(above), to.dot(forward));

        // Depth first: the camera's own forward, so the screen plane the next step composes on is
        // the one it will actually have.
        let near = (self.camera_distance - self.dead_zone_depth * 0.5).max(NEAREST);
        let far = (self.camera_distance + self.dead_zone_depth * 0.5).max(near);
        let depth = at.z.clamp(near, far);
        let mut owed = Vec3::new(0.0, 0.0, at.z - depth);

        let span = frame.lens.span(depth.max(NEAREST));
        // `frame.screen` already carries the lead, so one thing decides where the character sits
        // (#1330). Halves, because a zone is measured from its centre out.
        let seen = Vec2::new(at.x / span.x, at.y / span.y) - (self.screen_position + frame.screen);
        let dead = self.dead_zone * 0.5;
        let soft = self.soft_zone.max(self.dead_zone) * 0.5;
        // Moving the camera by `d` along an axis moves the target by `-d` on screen, so what is
        // owed in metres is the excess itself.
        let past = past(seen, dead);
        let share = Vec2::new(ramp(seen.x, dead.x, soft.x), ramp(seen.y, dead.y, soft.y));
        owed.x = past.x * share.x * span.x;
        owed.y = past.y * share.y * span.y;

        let eased = match fresh && self.center_on_activate {
            // Taking over: land on it rather than easing in from wherever the camera was left.
            true => owed,
            false => Vec3::new(
                owed.x * crate::virtual_camera::settled(dt, self.damping_value.x),
                owed.y * crate::virtual_camera::settled(dt, self.damping_value.y),
                owed.z * crate::virtual_camera::settled(dt, self.damping_value.z),
            ),
        };
        from + right * eased.x + above * eased.y + forward * eased.z
    }
}

/// The Body, when the vcam asks for it. Reached from
/// [`body_stage`](crate::virtual_camera::body_stage), never registered on its own — the Body has one
/// owner, as the Aim does.
///
/// Answers whether it ran, so the caller knows not to damp a position that is already eased.
pub fn body(step: &mut crate::rig::RigStep) -> bool {
    let Some(composer) = of(step.registry, step.entity) else {
        return false;
    };
    // No memory of this vcam means it was not planned last step: it is arriving.
    let fresh = step.carried.composed.of(step.entity).is_none();
    let placed = composer.placed(&step.frame, fresh, step.dt);
    step.memory
        .composed
        .set(step.entity, crate::framing::Framed::at(step.frame.target));
    step.frame.place(placed);
    true
}

#[cfg(test)]
mod tests;
