//! [`PostProcessVolume`] — a post-process stack that switches on where something is (#1222).

use crate::component::Component;
use crate::post_process::PostEffect;
use crate::reflect::FieldRange;

#[allow(unused_imports)]
use crate::Reflect;

/// Effects that join the frame's stack while a body is inside this volume's sensor.
///
/// The **shape is the collider** on the same entity, which must be a trigger: what is already the
/// engine's way of saying "this region", with its own groups to say who counts. A `global` volume
/// needs none and is always on.
///
/// 🔴 The blend fades **inward** from the collider's surface, where Unity's fades outward. The
/// sensor is what says a body arrived at all, so the surface is the first place a weight can be
/// asked for — fading outward would need a second, wider shape nobody authored.
#[derive(Debug, Clone, Reflect)]
#[reflect(category = "Rendering")]
pub struct PostProcessVolume {
    /// The effects this volume contributes, first to last.
    #[reflect(bare = "material")]
    pub effects: Vec<PostEffect>,
    /// On everywhere, with no shape and no trigger: the scene's base look.
    pub global: bool,
    /// Higher wins where volumes overlap — it is applied later, over what the lower ones left.
    pub priority: i32,
    /// How far inside the shape the volume reaches full strength, in metres. Zero cuts.
    #[reflect(range = BLEND_RANGE)]
    pub blend_distance: f32,
    /// A ceiling over every effect of this volume: 0.5 is half of whatever each one asked for.
    #[reflect(range = WEIGHT_RANGE)]
    pub weight: f32,
    /// Off contributes nothing, and costs nothing.
    pub enabled: bool,
}

const WEIGHT_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 1.0,
    step: 0.01,
};

const BLEND_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 100.0,
    step: 0.1,
};

impl Default for PostProcessVolume {
    fn default() -> Self {
        Self {
            effects: Vec::new(),
            global: false,
            priority: 0,
            blend_distance: 1.0,
            weight: 1.0,
            enabled: true,
        }
    }
}

impl Component for PostProcessVolume {}

impl PostProcessVolume {
    /// How much of this volume applies at `depth` metres inside its shape: none at the surface,
    /// all of it a blend distance in. A global volume is asked with [`f32::INFINITY`].
    pub fn weight_at(&self, depth: f32) -> f32 {
        if !self.enabled || depth < 0.0 {
            return 0.0;
        }
        let reached = match self.blend_distance > 0.0 {
            true => (depth / self.blend_distance).clamp(0.0, 1.0),
            // A cut, not a division by zero.
            false => 1.0,
        };
        // Smooth at both ends: a linear ramp starts and stops with a visible corner, and a
        // post-process fading in is exactly where that reads as a pop.
        let eased = reached * reached * (3.0 - 2.0 * reached);
        eased * self.weight.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests;
