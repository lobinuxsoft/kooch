//! The post-process a camera applies to what it rendered (#1201).

use kooch_core::Guid;

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// A material whose shader is `kind: post_process`, drawn over the finished frame.
///
/// 🔴 One per scene for now: the pass runs over the whole viewport, so two of them would need an
/// order and a reason. Without the component, or without a material, the frame is untouched.
#[derive(Debug, Clone, Reflect)]
#[reflect(category = "Rendering")]
pub struct PostProcess {
    /// The material to draw with. Its shader decides what the effect is.
    #[reflect(asset = "kooch_render::material::asset::Material")]
    pub material: Option<Guid>,
    /// Off leaves the frame as the camera rendered it, and costs nothing.
    pub enabled: bool,
}

impl Default for PostProcess {
    fn default() -> Self {
        Self {
            material: None,
            enabled: true,
        }
    }
}

impl Component for PostProcess {}
