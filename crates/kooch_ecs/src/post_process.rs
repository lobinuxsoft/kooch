//! The post-process a camera applies to what it rendered (#1201).

use kooch_core::Guid;

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Materials whose shaders are `kind: post_process`, drawn over the finished frame in list order —
/// each one reads what the one before it produced.
///
/// 🔴 One stack per scene: the effects run over the whole viewport, so the order has to be in one
/// place to be read at a glance. Without the component, or with it off, the frame is untouched.
#[derive(Debug, Clone, Reflect)]
#[reflect(category = "Rendering")]
pub struct PostProcess {
    /// The effects, first to last. An empty slot is skipped.
    #[reflect(asset = "kooch_render::material::asset::Material", alias = "material")]
    pub materials: Vec<Option<Guid>>,
    /// Off leaves the frame as the camera rendered it, and costs nothing.
    pub enabled: bool,
}

impl Default for PostProcess {
    fn default() -> Self {
        Self {
            materials: Vec::new(),
            enabled: true,
        }
    }
}

impl Component for PostProcess {}

#[cfg(test)]
mod tests;
