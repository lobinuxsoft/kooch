//! [`CameraFrame`] — the pose a rig is building, passed from stage to stage (#1331).
//!
//! 🔴 Every bug this rig has had was the same one: two things deciding one quantity. The framing
//! eased a position the vcam was damping; it turned a camera the player was turning; the lookahead
//! fed a point to two consumers; the wall moved a camera the framing then read as slack. None of
//! them were visible in the code, because each stage wrote to a local the next one happened to
//! read.
//!
//! So a pose is a **value** now. It arrives at a stage, the stage changes the one thing it owns,
//! and it goes on. What a stage may touch is written down here rather than remembered.

use glam::{Quat, Vec2, Vec3};

use crate::framing::Lens;

/// The order stages run in. Each owns one quantity, and nothing else may write it.
///
/// Named after Cinemachine's, because the division is the same and a second vocabulary for one idea
/// is how this went wrong in the first place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// Where the camera stands: the follow mode, its offset, its distance. Owns `position`.
    Body,
    /// Where it looks. Owns `rotation`.
    Aim,
    /// Where the target sits on the screen — a lateral offset to `position`, never its depth.
    Frame,
    /// The last word on `position`: a wall is not negotiable.
    Collide,
}

/// One vcam's pose in flight, and what every stage needs to answer about it.
#[derive(Debug, Clone, Copy)]
pub struct CameraFrame {
    /// Where the camera stands. [`Stage::Body`] decides it, [`Stage::Frame`] offsets it sideways,
    /// [`Stage::Collide`] has the last word.
    pub position: Vec3,
    /// Where it looks. [`Stage::Aim`]'s, and nobody else's — in a third-person rig this belongs to
    /// whoever is holding the stick.
    pub rotation: Quat,
    /// 🔴 Where the body wanted to stand, before a frame or a wall moved it. Carried so a later
    /// stage never has to ask "was I pushed?" — the framing had to be handed this by hand, and the
    /// next stage would have had to be told too (#1330).
    pub free: Vec3,
    /// The target this vcam is following, in world space.
    pub target: Vec3,
    /// Where the target is held on screen: `0` centre, `±0.5` the edges. A lead shifts this rather
    /// than moving `target`, so one thing decides where the character sits (#1330).
    pub screen: Vec2,
    /// Which way is up for this vcam, and the yaw origin carried against the pole.
    pub up: Vec3,
    pub reference: Vec3,
    /// The screen every zone is measured against.
    pub lens: Lens,
}

impl CameraFrame {
    /// A frame standing where the body put it, looking where it looked.
    pub fn new(
        position: Vec3,
        rotation: Quat,
        target: Vec3,
        up: Vec3,
        reference: Vec3,
        lens: Lens,
    ) -> Self {
        Self {
            position,
            rotation,
            free: position,
            target,
            screen: Vec2::ZERO,
            up,
            reference,
            lens,
        }
    }

    /// The camera's own axes: right, up and forward.
    pub fn axes(&self) -> (Vec3, Vec3, Vec3) {
        (
            self.rotation * Vec3::X,
            self.rotation * Vec3::Y,
            self.rotation * -Vec3::Z,
        )
    }

    /// Moves the camera without touching where the body wanted it — what [`Stage::Frame`] and
    /// [`Stage::Collide`] do, and what makes `free` worth carrying.
    pub fn displace(&mut self, position: Vec3) {
        self.position = position;
    }

    /// Where `point` lands on screen, as a fraction from where the target is held.
    pub fn seen(&self, point: Vec3) -> Option<Vec2> {
        let (right, above, forward) = self.axes();
        let offset = point - self.position;
        let depth = offset.dot(forward);
        if depth <= 0.0 {
            return None;
        }
        let span = self.lens.span(depth);
        Some(Vec2::new(offset.dot(right) / span.x, offset.dot(above) / span.y) - self.screen)
    }
}

#[cfg(test)]
mod tests;
