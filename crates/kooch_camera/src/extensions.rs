//! [`CameraOffset`] and [`CameraRecomposer`] — the last word over a rig that got it nearly right:
//! Cinemachine's `CinemachineCameraOffset` and `CinemachineRecomposer` (#1371).
//!
//! Both are **extensions** there and stages here, which is what [`RigStage`] was built for (#1331):
//! they name where they run and register at it, and nothing in the rig changes to accept them. The
//! first real users of that extension point.

use glam::{Vec2, Vec3};
use kooch_ecs::Reflect;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::{FieldChoice, FieldRange};

use crate::rig::{RigStage, RigStep};

/// Which stage an extension runs after. The same list [`RigStage`] declares, as a field.
pub static AFTER_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "Lead",
        value: 0,
    },
    FieldChoice {
        label: "Body",
        value: 1,
    },
    FieldChoice {
        label: "Frame",
        value: 2,
    },
    FieldChoice {
        label: "Collide",
        value: 3,
    },
    FieldChoice {
        label: "Aim",
        value: 4,
    },
];

/// The stage `apply_after` names.
fn after(at: u32) -> RigStage {
    match at {
        0 => RigStage::Lead,
        1 => RigStage::Body,
        2 => RigStage::Frame,
        3 => RigStage::Collide,
        _ => RigStage::Aim,
    }
}

/// Adds a final offset to the camera, in its own frame.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Camera")]
pub struct CameraOffset {
    /// Read in the camera's own frame, whose up is the vcam's: an offset authored as "up" stays up
    /// on the side of a planet.
    pub offset: Vec3,
    /// Which stage it runs after.
    #[reflect(choices = AFTER_CHOICES)]
    pub apply_after: u32,
    /// Turn back afterwards so the target keeps the place on screen it had. Only after the Aim, when
    /// there is a rotation to preserve anything with.
    pub preserve_composition: bool,
}

impl Default for CameraOffset {
    fn default() -> Self {
        Self {
            offset: Vec3::ZERO,
            apply_after: 4,
            preserve_composition: false,
        }
    }
}

impl Component for CameraOffset {}

/// A hand-made tweak over what the rig decided — the hook a Timeline drives (#1354).
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Camera")]
pub struct CameraRecomposer {
    /// Degrees about the camera's own right, positive looking up.
    #[reflect(range = ANGLE_RANGE)]
    pub tilt: f32,
    /// Degrees about the vcam's up — the gravity-aligned one, so a pan authored on the side of a
    /// planet turns along its horizon.
    #[reflect(range = ANGLE_RANGE)]
    pub pan: f32,
    /// Which stage it runs after.
    #[reflect(choices = AFTER_CHOICES)]
    pub apply_after: u32,
}

const ANGLE_RANGE: FieldRange = FieldRange {
    min: -180.0,
    max: 180.0,
    step: 0.5,
};

impl Default for CameraRecomposer {
    fn default() -> Self {
        Self {
            tilt: 0.0,
            pan: 0.0,
            apply_after: 4,
        }
    }
}

impl Component for CameraRecomposer {}

/// What `entity` carries, for the stage it named.
fn of<T: Component + Copy>(registry: &ComponentRegistry, entity: Entity) -> Option<T> {
    registry.get_cpu::<T>()?.get(entity).copied()
}

/// Runs both extensions that named `stage`, in Cinemachine's order: the offset moves, then the
/// recomposer turns.
fn applied(step: &mut RigStep, stage: RigStage) {
    if let Some(offset) = of::<CameraOffset>(step.registry, step.entity)
        && after(offset.apply_after) == stage
    {
        offset.apply(step, stage);
    }
    if let Some(recomposer) = of::<CameraRecomposer>(step.registry, step.entity)
        && after(recomposer.apply_after) == stage
    {
        recomposer.apply(step);
    }
}

impl CameraOffset {
    fn apply(&self, step: &mut RigStep, stage: RigStage) {
        let frame = &mut step.frame;
        // Where the target sits now, so it can be put back there. Only after the Aim: before it
        // there is no rotation of this step's to preserve.
        let held = (self.preserve_composition && stage == RigStage::Aim).then(|| {
            crate::framing::seen_at(frame.rotation, frame.target - frame.position, step.up)
        });
        frame.displace(frame.position + frame.rotation * self.offset);
        if let Some(held) = held {
            // Aim at the target from where it now stands, then put it back off centre by what it
            // was off by — Cinemachine's `LookRotation` then `ApplyCameraRotation(-screenOffset)`.
            let aimed = crate::virtual_camera::look_at(
                frame.position,
                frame.target,
                step.up,
                step.reference,
            );
            frame.rotation = crate::framing::turned(aimed, -held, step.up);
        }
    }
}

impl CameraRecomposer {
    fn apply(&self, step: &mut RigStep) {
        if self.tilt == 0.0 && self.pan == 0.0 {
            return;
        }
        // Tilt about the camera's own right, pan about the vcam's up: Cinemachine's order, and the
        // one that leaves the horizon level.
        step.frame.rotation = crate::framing::turned(
            step.frame.rotation,
            Vec2::new(-self.pan, self.tilt),
            step.up,
        );
    }
}

/// One entry point per stage: a [`RigFn`](crate::rig::RigFn) is a plain pointer and cannot carry
/// which stage it was registered at, so the stage is in the function instead of in a field.
pub fn after_lead(step: &mut RigStep) {
    applied(step, RigStage::Lead);
}

/// The same, after the Body.
pub fn after_body(step: &mut RigStep) {
    applied(step, RigStage::Body);
}

/// The same, after the Frame.
pub fn after_frame(step: &mut RigStep) {
    applied(step, RigStage::Frame);
}

/// The same, after the Collide.
pub fn after_collide(step: &mut RigStep) {
    applied(step, RigStage::Collide);
}

/// The same, after the Aim.
pub fn after_aim(step: &mut RigStep) {
    applied(step, RigStage::Aim);
}

#[cfg(test)]
mod tests;
