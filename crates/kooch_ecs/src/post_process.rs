//! The post-process a camera applies to what it rendered (#1201).

use kooch_core::Guid;

use crate::component::Component;
use crate::reflect::FieldRange;

#[allow(unused_imports)]
use crate::Reflect;

/// Effects drawn over the finished frame in list order — each one reads what the one before it
/// produced.
///
/// 🔴 One stack per scene: the effects run over the whole viewport, so the order has to be in one
/// place to be read at a glance. Without the component, or with it off, the frame is untouched.
#[derive(Debug, Clone, Reflect)]
#[reflect(category = "Rendering")]
pub struct PostProcess {
    /// The effects, first to last.
    #[reflect(alias = "materials, material", bare = "material")]
    pub effects: Vec<PostEffect>,
    /// Off leaves the frame as the camera rendered it, and costs nothing.
    pub enabled: bool,
}

impl Default for PostProcess {
    fn default() -> Self {
        Self {
            effects: Vec::new(),
            enabled: true,
        }
    }
}

impl Component for PostProcess {}

/// One entry of the stack: a material whose shader is `kind: post_process`, and how much of it
/// shows (#1209).
#[derive(Debug, Clone, PartialEq, Reflect)]
pub struct PostEffect {
    /// A material whose shader is `kind: post_process`. An empty slot is skipped.
    #[reflect(asset = "kooch_render::material::asset::Material")]
    pub material: Option<Guid>,
    /// Off skips this effect alone, and costs nothing.
    pub enabled: bool,
    /// How much of the effect mixes over what it read: 0 is none of it and costs nothing, 1 is all
    /// of it.
    #[reflect(range = WEIGHT_RANGE)]
    pub weight: f32,
}

const WEIGHT_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 1.0,
    step: 0.01,
};

impl Default for PostEffect {
    fn default() -> Self {
        Self {
            material: None,
            enabled: true,
            weight: 1.0,
        }
    }
}

impl PostEffect {
    /// The material when this effect draws anything: on, weighted above zero, and assigned.
    pub fn drawn(&self) -> Option<Guid> {
        (self.enabled && self.weight > 0.0)
            .then_some(self.material)
            .flatten()
    }
}

#[cfg(test)]
mod tests;
