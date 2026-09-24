//! Mesh renderer component.

use kooch_core::Guid;

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Component that binds an entity to a mesh and a material for rendering.
#[derive(Debug, Clone, Reflect)]
#[reflect(category = "Rendering")]
pub struct MeshRenderer {
    /// When `Some`, the meshlet pipeline picks this entity up via the scene cull. Persistent across
    /// runs — the GUID lives in the asset's `.meta` sidecar. The inspector renders this as a typed
    /// dropdown picker that lists every `MeshletMesh` the `AssetDatabase` has registered.
    #[reflect(asset = "kooch_render::meshlet::asset::MeshletMesh")]
    pub mesh: Option<Guid>,
    /// When `Some`, the meshlet scene system resolves the GUID to a `MaterialPool` slot via the
    /// `MaterialPipeline` resource. The inspector renders this as a typed dropdown of every
    /// `Material` registered with the asset database.
    #[reflect(asset = "kooch_render::material::asset::Material")]
    pub material: Option<Guid>,
    /// Whether this renderer is drawn.
    pub visible: bool,
    /// Whether this renderer casts shadows.
    pub cast_shadows: bool,
    /// Whether this renderer receives shadows.
    pub receive_shadows: bool,
    /// The layer this renderer is in (#1307). A camera, a light or a shadow view keeps only what
    /// its own mask meets — the mask belongs to whoever is choosing, and the object answers what
    /// it is. "Default" is where everything starts.
    #[reflect(layer)]
    pub layer: u32,
    /// The mask this renderer carried before it named one layer, kept so a scene still loads it and
    /// never drawn: a mesh is in one layer, and a second place to say so is a second place for it
    /// to disagree (#1307).
    #[reflect(hidden)]
    pub layers: u32,
}

impl MeshRenderer {
    /// Which layer this renderer is in, reading a pre-#1307 mask when that is all it has: the
    /// lowest bit it claimed, which is the layer an author picked in every project that used one.
    pub fn layer(&self) -> u32 {
        if self.layer != 0 || self.layers == kooch_core::layers::DEFAULT_LAYER {
            return self.layer.min(31);
        }
        self.layers.trailing_zeros().min(31)
    }

    /// What a camera's mask is tested against: the one bit this renderer is in.
    pub fn layer_mask(&self) -> u32 {
        1 << self.layer()
    }
}

impl Default for MeshRenderer {
    fn default() -> Self {
        Self {
            mesh: None,
            material: None,
            visible: true,
            cast_shadows: true,
            receive_shadows: true,
            layer: 0,
            layers: kooch_core::layers::DEFAULT_LAYER,
        }
    }
}

impl Component for MeshRenderer {}

#[cfg(test)]
mod tests;
