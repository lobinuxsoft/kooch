//! [`CameraBrain`] — which rendering camera the virtual cameras drive (#1221).

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Marks the camera a virtual camera writes its pose onto. Cinemachine's Brain, and the answer to
/// "which camera is this rig for": a scene with a base camera and an overlay has two, and picking
/// the highest-priority one is a guess that changes the moment an overlay outranks the base.
///
/// With no brain anywhere the elected vcam falls back to the highest-priority active camera that is
/// not an overlay, so a scene authored before this keeps moving.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Camera")]
pub struct CameraBrain {
    /// A brain that is switched off is not a candidate, and its camera stays where it was put.
    pub enabled: bool,
}

impl Default for CameraBrain {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl Component for CameraBrain {}

#[cfg(test)]
mod tests;
