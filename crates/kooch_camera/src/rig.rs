//! The rig — the ordered list of stages a pose passes through, and what each one may decide
//! (#1331).
//!
//! 🔴 The order used to be the order of statements in one 200-line function, and a stage read
//! whatever local the stage above it happened to leave behind. Four bugs in a row were that same
//! shape: two things deciding one quantity. Here a stage is registered at a [`RigStage`], is handed
//! the pose, changes the single quantity it owns, and hands it on — so a new component (an orbit
//! input, a noise, a timeline) plugs in without anyone editing the loop.

use glam::Vec3;
use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;

use crate::frame::CameraFrame;
use crate::framing::Tracked;
use crate::lookahead::Leads;
use crate::occlusion::Arms;
use crate::target::GroupPose;
use crate::virtual_camera::{VirtualCamera, seed_reference, transported};

/// Where a stage runs, and by running there what it is allowed to decide. Declared in order: a
/// stage never reads a later one's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RigStage {
    /// How far ahead of the target the rig looks. Owns `target` and `screen`.
    Lead,
    /// Where the camera stands. Owns `position` and `free`.
    Body,
    /// Where the target sits on screen. Moves `position` sideways, never in depth.
    Frame,
    /// The last word on `position`: a wall pulls the camera in.
    Collide,
    /// Where the camera looks. Owns `rotation` — in a third-person rig, the player's.
    Aim,
}

/// One stage's work on one vcam's pose.
pub type RigFn = fn(&mut RigStep);

/// The stages, in order. Each component's plugin registers its own, which is what makes "one owner
/// per quantity" a thing the rig is built out of rather than a comment.
#[derive(Debug, Clone, Default)]
pub struct CameraRig {
    stages: Vec<(RigStage, RigFn)>,
}

impl CameraRig {
    /// The stages a camera rig is made of, in the order a pose passes through them.
    pub fn standard() -> Self {
        let mut rig = Self::default();
        rig.add(RigStage::Lead, crate::lookahead::lead_stage);
        rig.add(RigStage::Body, crate::virtual_camera::body_stage);
        rig.add(RigStage::Frame, crate::framing::frame_stage);
        rig.add(RigStage::Collide, crate::occlusion::collide_stage);
        rig.add(RigStage::Aim, crate::virtual_camera::aim_stage);
        rig
    }

    /// Adds a stage, keeping the list in [`RigStage`] order. Two at the same stage run in the order
    /// they were added.
    pub fn add(&mut self, stage: RigStage, run: RigFn) {
        let at = self.stages.partition_point(|(other, _)| *other <= stage);
        self.stages.insert(at, (stage, run));
    }

    /// Runs every stage over one pose.
    pub fn run(&self, step: &mut RigStep) {
        for (_, run) in &self.stages {
            run(step);
        }
    }
}

/// Everything a stage may read, and the pose it is changing.
pub struct RigStep<'a> {
    /// The pose in flight. A stage writes the one quantity its [`RigStage`] owns.
    pub frame: CameraFrame,
    /// The vcam being posed.
    pub entity: Entity,
    /// How it was authored.
    pub vcam: &'a VirtualCamera,
    /// The real target: a group's centre, and the member whose rotation stands for the group.
    /// 🔴 Not [`CameraFrame::target`], which is the point being framed — a lead moves that one.
    pub target: GroupPose,
    /// Which way is up for this vcam.
    pub up: Vec3,
    /// The yaw origin it measures from, carried between steps.
    pub reference: Vec3,
    /// The fixed step, so damping is deterministic.
    pub dt: f32,
    /// For a stage that asks another crate a question, as the collision asks physics.
    pub resources: &'a Resources,
    /// For a stage that reads its own component off `entity`.
    pub registry: &'a ComponentRegistry,
    /// What the stages left last step.
    pub carried: &'a RigMemory,
    /// What they are leaving this one.
    pub memory: &'a mut RigMemory,
}

/// What the stages carry between steps. Runtime state, never authored, and rebuilt from the vcams
/// seen each step so a despawned one leaves nothing behind.
#[derive(Debug, Clone, Default)]
pub struct RigMemory {
    /// Each vcam's yaw origin.
    pub horizons: Horizons,
    /// Each vcam's arm: where the rig would have it, and any return in progress.
    pub arms: Arms,
    /// Each framing's slack.
    pub tracked: Tracked,
    /// Each lead's offset.
    pub leads: Leads,
}

/// The yaw origin each vcam measures from, carried between steps.
#[derive(Debug, Clone, Default)]
pub struct Horizons {
    /// Per vcam: the up it last used, and the reference it carried.
    frames: std::collections::HashMap<Entity, (Vec3, Vec3)>,
}

impl Horizons {
    /// This vcam's yaw origin on a new up, carried from the last. A first step seeds it from a world
    /// axis: by the hairy ball theorem no reference derived from `up` alone is continuous, so one
    /// derived every step would swing the camera half a turn as a target rolls over a pole.
    pub fn carry(&self, vcam: Entity, up: Vec3) -> Vec3 {
        match self.frames.get(&vcam) {
            Some((last_up, reference)) => transported(*reference, *last_up, up),
            None => seed_reference(up),
        }
    }

    /// Remembers what this vcam used, for the next step to carry from.
    pub fn set(&mut self, vcam: Entity, up: Vec3, reference: Vec3) {
        self.frames.insert(vcam, (up, reference));
    }
}

#[cfg(test)]
mod tests;
