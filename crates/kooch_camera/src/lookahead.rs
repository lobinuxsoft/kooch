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

/// Leads the target of the vcam it sits on along its velocity. Beside a [`VirtualCamera`]; with a
/// [`RotationComposer`] the led point is what gets framed.
///
/// [`VirtualCamera`]: crate::VirtualCamera
/// [`RotationComposer`]: crate::RotationComposer
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
    pub smoothing_time: f32,
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
            smoothing_time: 0.6,
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
}

impl Lead {
    /// How far ahead of its target the lead is holding the frame, in metres. What a gizmo draws:
    /// the number in the Inspector is what was asked for, this is what is happening.
    pub fn offset(&self) -> Vec3 {
        self.offset
    }

    /// Starting on a target at `at`, not moving.
    pub fn at(at: Vec3) -> Self {
        Self {
            last: at,
            offset: Vec3::ZERO,
        }
    }
}

impl CameraLookahead {
    /// The point to frame this step: `target` plus the lead its motion since the last step asks
    /// for, tweened and capped.
    pub fn offset(&self, lead: &mut Lead, target: Vec3, up: Vec3, dt: f32) -> Vec3 {
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
        // Exponential, like the rig's damping: the goal moves with the target's speed every step,
        // and a tween that restarts on a moving goal steps whenever the running starts or stops
        // (#1336).
        let alpha = crate::virtual_camera::settled(dt, self.smoothing_time);
        lead.offset += (goal - lead.offset) * alpha;
        lead.offset
    }
}

/// Every leading vcam's state, carried between steps and rebuilt from the vcams seen, so a
/// despawned one leaves nothing behind.
#[derive(Debug, Clone, Default)]
pub struct Leads(pub(crate) HashMap<Entity, Lead>);

impl Leads {
    /// The lead a vcam is carrying, for anything that draws it.
    pub fn of(&self, vcam: Entity) -> Option<Lead> {
        self.0.get(&vcam).copied()
    }

    /// Remembers this vcam's lead for the next step.
    pub fn set(&mut self, vcam: Entity, lead: Lead) {
        self.0.insert(vcam, lead);
    }
}

/// The Lead stage: how far ahead of the target the rig looks.
///
/// 🔴 An offset, not a point. With a framing it shifts where the target is HELD on screen; without
/// one it moves what the rig follows — the same idea said in the only vocabulary each case has.
/// Applying it in both places is two things deciding where the character sits (#1330).
pub fn lead_stage(step: &mut crate::rig::RigStep) {
    let Some(lookahead) = step
        .registry
        .get_cpu::<CameraLookahead>()
        .and_then(|storage| storage.get(step.entity))
        .filter(|lookahead| lookahead.enabled)
    else {
        return;
    };
    let mut lead = step
        .carried
        .leads
        .of(step.entity)
        .unwrap_or_else(|| Lead::at(step.frame.target));
    let offset = lookahead.offset(&mut lead, step.frame.target, step.up, step.dt);
    step.memory.leads.set(step.entity, lead);
    match crate::framing::of(step.registry, step.entity) {
        Some(_) => step.frame.hold(offset),
        None => step.frame.target += offset,
    }
}

#[cfg(test)]
mod tests;
