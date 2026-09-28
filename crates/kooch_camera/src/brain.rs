//! [`CameraBrain`] — which rendering camera the virtual cameras drive (#1221).

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Marks the camera a virtual camera writes its pose onto. Cinemachine's Brain, and the answer to
/// "which camera is this rig for": a scene with a base camera and an overlay has two, and picking
/// the highest-priority one is a guess that changes the moment an overlay outranks the base.
///
/// 🔴 Required. A scene where no camera carries an enabled brain is driven by nothing, and the rig
/// says so once in the log rather than moving a camera nobody pointed it at. Several brains order by
/// the camera's `priority`.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Camera")]
pub struct CameraBrain {
    /// A brain that is switched off is not a candidate, and its camera stays where it was put.
    pub enabled: bool,
    /// How long a change of virtual camera takes, in seconds. Zero is a cut.
    ///
    /// 🔴 The brain's, not a vcam's. A blend is between **two** of them, so asking one of them how
    /// long it takes only raises "which one?" — ours answered "the incoming", which is a
    /// convention rather than an answer. Cinemachine keeps it here for the same reason, and the
    /// per-pair table belongs here too when it comes (#1339).
    #[reflect(range = BLEND_RANGE)]
    pub blend_duration: f32,
    /// The shape of that change.
    #[reflect(choices = kooch_ecs::tween::CURVE_CHOICES)]
    pub blend_curve: u32,
    /// Which end of it is slow.
    #[reflect(choices = kooch_ecs::tween::EASE_CHOICES)]
    pub blend_ease: u32,
}

/// Long enough to read as a transition, short enough not to feel like the game took the camera.
const BLEND_RANGE: kooch_ecs::reflect::FieldRange = kooch_ecs::reflect::FieldRange {
    min: 0.0,
    max: 5.0,
    step: 0.05,
};

impl Default for CameraBrain {
    fn default() -> Self {
        Self {
            enabled: true,
            blend_duration: 0.5,
            blend_curve: kooch_ecs::tween::CURVE_SINE,
            blend_ease: kooch_ecs::tween::EASE_IN_OUT,
        }
    }
}

impl Component for CameraBrain {}

#[cfg(test)]
mod tests;
