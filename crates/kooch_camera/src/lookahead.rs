//! [`CameraLookahead`] — framing where the target is going, not where it is: Cinemachine's
//! Lookahead (#1253).
//!
//! The velocity comes from the target's own recent positions, so a rig leads a target nothing
//! simulates as well as a body.

use std::collections::HashMap;

use glam::Vec3;
use kooch_ecs::Reflect;
use kooch_ecs::component::Component;
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::FieldRange;
use kooch_ecs::tween::Chase;

/// Leads the target of the vcam it sits on along its velocity. Beside a [`VirtualCamera`]; with a
/// [`CameraFraming`] the led point is what gets framed.
///
/// [`VirtualCamera`]: crate::VirtualCamera
/// [`CameraFraming`]: crate::CameraFraming
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Camera")]
pub struct CameraLookahead {
    /// Off frames the target itself.
    pub enabled: bool,
    /// How far ahead to look, in seconds of the target's current velocity: at 6 m/s, 0.5 leads by
    /// 3 m.
    #[reflect(range = LEAD_RANGE)]
    pub lead_time: f32,
    /// Seconds the lead takes to reach a new velocity's offset once it holds — exactly, a tween, so
    /// stopping returns the framing without a snap. Zero follows every change at once.
    #[reflect(range = LEAD_RANGE)]
    pub smoothing_duration: f32,
    /// The furthest the lead goes, in metres: a teleport is a velocity too.
    #[reflect(range = DISTANCE_RANGE)]
    pub max_distance: f32,
    /// Leads only across the ground, so a jump does not swing the camera down and up each arc.
    /// "Up" is the vcam's own.
    pub ignore_vertical: bool,
}

const LEAD_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 3.0,
    step: 0.05,
};

const DISTANCE_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 20.0,
    step: 0.1,
};

impl Default for CameraLookahead {
    fn default() -> Self {
        Self {
            enabled: true,
            lead_time: 0.4,
            smoothing_duration: 0.6,
            max_distance: 4.0,
            ignore_vertical: true,
        }
    }
}

impl Component for CameraLookahead {}

/// One vcam's lead in flight: where the target was last step, and the offset's tween.
#[derive(Debug, Clone, Copy)]
pub struct Lead {
    last: Vec3,
    offset: Vec3,
    chase: Chase<Vec3>,
}

impl Lead {
    /// Starting on a target at `at`, not moving.
    pub fn at(at: Vec3) -> Self {
        Self {
            last: at,
            offset: Vec3::ZERO,
            chase: Chase::at(Vec3::ZERO),
        }
    }
}

impl CameraLookahead {
    /// The point to frame this step: `target` plus the lead its motion since the last step asks
    /// for, tweened and capped.
    pub fn led(&self, lead: &mut Lead, target: Vec3, up: Vec3, dt: f32) -> Vec3 {
        let velocity = match dt > 0.0 {
            true => (target - lead.last) / dt,
            false => Vec3::ZERO,
        };
        lead.last = target;
        let velocity = match self.ignore_vertical {
            true => velocity - up * velocity.dot(up),
            false => velocity,
        };
        let goal =
            (velocity * self.lead_time.max(0.0)).clamp_length_max(self.max_distance.max(0.0));
        lead.offset = lead
            .chase
            .step(lead.offset, goal, dt, self.smoothing_duration);
        target + lead.offset
    }
}

/// Every leading vcam's state, carried between steps and rebuilt from the vcams seen, so a
/// despawned one leaves nothing behind.
#[derive(Debug, Clone, Default)]
pub struct Leads(pub(crate) HashMap<Entity, Lead>);

#[cfg(test)]
mod tests;
