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
}

impl Default for MeshRenderer {
    fn default() -> Self {
        Self {
            mesh: None,
            material: None,
            visible: true,
            cast_shadows: true,
            receive_shadows: true,
        }
    }
}

impl Component for MeshRenderer {}

#[cfg(test)]
mod tests;
