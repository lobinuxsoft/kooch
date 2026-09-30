//! [`ThirdPersonFollow`] — the body that stands the camera over a shoulder: Cinemachine's
//! `CinemachineThirdPersonFollow` (#1391).
//!
//! 🔴 A body is a component and its own fields live on it. These were on `VirtualCamera`, where they
//! showed on a body that could not read them and needed a condition to hide — while the rings, the
//! same situation, had a component of their own. One question, two answers.
//!
//! The chain is `target → shoulder → hand → camera`: the shoulder in the **levelled** basis, the arm
//! along the view's own up, which pitches (#1365).

use glam::Vec3;
use kooch_ecs::Reflect;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::FieldRange;

/// The body of a vcam whose `follow` is [`Third Person Follow`](crate::FOLLOW_SHOULDER).
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Camera")]
pub struct ThirdPersonFollow {
    /// Where the arm's pivot sits, offset from the target in the arm's own basis: `x` beside the
    /// view, `y` along `up`, `z` the way the camera looks.
    ///
    /// 🔴 The arm's basis, not the target's. Cinemachine's offsets in the target's, which it can
    /// because its rotation IS the target's; here the orbit owns the yaw, so a shoulder in the
    /// character's frame would swing around it as the player looks about (#1359).
    pub shoulder_offset: Vec3,
    /// How far the arm's pivot sits above the shoulder.
    ///
    /// 🔴 Not the same axis as `shoulder_offset.y`, which #1359 assumed and was wrong about: the
    /// shoulder sits in the **levelled** basis and this along the view's own up, which pitches. Look
    /// down and the pivot swings forward and down while the shoulder stays — it is what decides how
    /// the target's place on screen moves as the view turns vertically (#1365).
    pub vertical_arm_length: f32,
    /// Which shoulder the camera is on: `0` the left, `1` the right, halfway between them centred.
    #[reflect(range = SIDE_RANGE)]
    pub camera_side: f32,
    /// How far behind the hand the camera sits.
    #[reflect(range = DISTANCE_RANGE)]
    pub camera_distance: f32,
}

/// Both shoulders and everywhere between them.
const SIDE_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 1.0,
    step: 0.01,
};

const DISTANCE_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 100.0,
    step: 0.1,
};

impl Default for ThirdPersonFollow {
    /// Cinemachine's own, so a rig authored against its documentation lands where it says.
    fn default() -> Self {
        Self {
            shoulder_offset: Vec3::new(0.5, -0.4, 0.0),
            vertical_arm_length: 0.4,
            camera_side: 1.0,
            camera_distance: 2.0,
        }
    }
}

impl Component for ThirdPersonFollow {}

/// The body on `vcam`, if it has one. The only place that question is asked.
pub fn of(registry: &ComponentRegistry, vcam: Entity) -> Option<ThirdPersonFollow> {
    registry.get_cpu::<ThirdPersonFollow>()?.get(vcam).copied()
}

impl ThirdPersonFollow {
    /// The pivot's offset from the target, read in the arm's basis. Built off `back` alone, so
    /// pitching the view does not roll the shoulder.
    pub fn shouldered(&self, back: Vec3, up: Vec3) -> Vec3 {
        let forward = -back;
        // `forward × up`, the same hand `look_at` builds its basis with.
        let right = forward.cross(up);
        // `Lerp(-x, x, side)`, written as the multiplier it is.
        let beside = self.shoulder_offset.x * (self.camera_side.clamp(0.0, 1.0) * 2.0 - 1.0);
        right * beside + up * self.shoulder_offset.y + forward * self.shoulder_offset.z
    }
}

#[cfg(test)]
mod tests;
