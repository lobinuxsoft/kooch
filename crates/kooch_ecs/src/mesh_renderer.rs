//! Mesh renderer component.
//!
//! Tags an entity as renderable with traditional polygonal geometry
//! (non-SDF). The render pipeline iterates entities with `MeshRenderer`
//! plus [`Transform`](crate::Transform) to draw meshes.

use kooch_core::Guid;

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Component that binds an entity to a mesh and a material for rendering.
///
/// Both `mesh` and `material` reference assets by their persistent
/// [`Guid`] — the same identifier the asset's `.meta` sidecar carries.
/// The render pipeline resolves each GUID through the `AssetServer` +
/// `AssetDatabase` resources at sync time. Storing the GUID (not a
/// runtime `Handle<T>`) keeps the component **persistible**: scene
/// serialization round-trips cleanly across runs and the reference
/// stays valid even if the source file is moved (as long as its
/// sidecar follows it).
///
/// # Default
///
/// - `mesh`: `None`
/// - `material`: `None`
/// - `visible`: true
/// - `cast_shadows`: true
/// - `receive_shadows`: true
#[derive(Debug, Clone, Reflect)]
#[reflect(category = "Rendering")]
pub struct MeshRenderer {
    /// When `Some`, the meshlet pipeline picks this entity up via the
    /// scene cull. Persistent across runs — the GUID lives in the
    /// asset's `.meta` sidecar. The inspector renders this as a typed
    /// dropdown picker that lists every `MeshletMesh` the
    /// `AssetDatabase` has registered.
    ///
    /// The string MUST match `std::any::type_name::<T>()` of the
    /// pointed-at asset, not the re-exported path: the asset server
    /// records the canonical type name on the sidecar, and the
    /// inspector picker filters entries by exact match.
    #[reflect(asset = "kooch_render::meshlet::asset::MeshletMesh")]
    pub mesh: Option<Guid>,
    /// When `Some`, the meshlet scene system resolves the GUID to a
    /// `MaterialPool` slot via the `MaterialPipeline` resource. The
    /// inspector renders this as a typed dropdown of every
    /// `Material` registered with the asset database. `None` falls
    /// back to the white-diffuse default at slot 0.
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
