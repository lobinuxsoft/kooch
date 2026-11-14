//! Sky renderer component.

use kooch_core::Guid;

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Component that marks an entity as a sky background source.
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Rendering")]
pub struct SkyRenderer {
    /// Whether this sky is active. Inactive skies are ignored by the renderer.
    pub active: bool,
    /// Priority for multi-sky tiebreaking (higher wins).
    pub priority: i32,
    /// A material whose shader is `kind: sky`. `None`, or one that cannot draw, shows the engine's
    /// own sky (#1900).
    #[reflect(asset = "kooch_render::material::asset::Material")]
    pub material: Option<Guid>,
}

impl Default for SkyRenderer {
    fn default() -> Self {
        Self {
            active: true,
            priority: 0,
            material: None,
        }
    }
}

impl Component for SkyRenderer {}

#[cfg(test)]
mod tests;
