//! [`CameraFrame`] — the pose a rig is building, passed from stage to stage (#1331).
//!
//! 🔴 Every bug this rig has had was the same one: two things deciding one quantity. The framing
//! eased a position the vcam was damping; it fought the `look_at` for the rotation, and was moved to
//! the position rather than given it (#1361); the lookahead fed a point to two consumers; the wall
//! moved a camera the framing then read as slack. None of them were visible in the code, because
//! each stage wrote to a local the next one happened to read.
//!
//! So a pose is a **value** now. It arrives at a stage, the stage changes the one thing it owns,
//! and it goes on. What a stage may touch is written down here rather than remembered.

use glam::{Quat, Vec2, Vec3};

use crate::framing::Lens;

/// One vcam's pose in flight, and what every stage needs to answer about it.
#[derive(Debug, Clone, Copy)]
pub struct CameraFrame {
    /// Where the camera stands. [`Body`] decides it and [`Collide`] has the last word.
    ///
    /// [`Body`]: crate::rig::RigStage::Body
    /// [`Collide`]: crate::rig::RigStage::Collide
    pub position: Vec3,
    /// Where it looks. [`Aim`](crate::rig::RigStage::Aim)'s, and nobody else's — one of the vcam's
    /// `look_at` modes, never two of them (#1361).
    pub rotation: Quat,
    /// 🔴 Where the body wanted to stand, before a frame or a wall moved it. Carried so a later
    /// stage never has to ask "was I pushed?" — the framing had to be handed this by hand, and the
    /// next stage would have had to be told too (#1330).
    pub free: Vec3,
    /// Where the camera stood last step, before that step's wall. A stage that measures how far it
    /// has drifted reads this rather than remembering its own answer, which is a second opinion
    /// about where the camera is.
    pub previous: Vec3,
    /// The target this vcam is following, in world space.
    pub target: Vec3,
    /// Where the target is held on screen: `0` centre, `±0.5` the edges. A lead shifts this rather
    /// than moving `target`, so one thing decides where the character sits (#1330).
    pub screen: Vec2,
    /// The screen every zone is measured against.
    pub lens: Lens,
}

impl CameraFrame {
    /// A frame standing where the body put it, looking where it looked.
    pub fn new(position: Vec3, previous: Vec3, rotation: Quat, target: Vec3, lens: Lens) -> Self {
        Self {
            position,
            rotation,
            free: position,
            previous,
            target,
            screen: Vec2::ZERO,
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

    /// Where the body stands the camera: the answer every later stage measures its own against.
    pub fn place(&mut self, position: Vec3) {
        self.position = position;
        self.free = position;
    }

    /// Moves the camera without touching where the body wanted it — what the
    /// [`Collide`](crate::rig::RigStage::Collide) stage does, and what makes `free` worth carrying.
    pub fn displace(&mut self, position: Vec3) {
        self.position = position;
    }

    /// Holds the target this far off centre, given as a world offset.
    ///
    /// 🔴 A lead moves where the character sits on screen, not what the rig follows. Said in both
    /// vocabularies it is two things deciding the same thing (#1330).
    pub fn hold(&mut self, offset: Vec3) {
        let (right, above, forward) = self.axes();
        let span = self
            .lens
            .span((self.target - self.position).dot(forward).max(0.01));
        self.screen = -Vec2::new(offset.dot(right) / span.x, offset.dot(above) / span.y);
    }
}

#[cfg(test)]
mod tests;
