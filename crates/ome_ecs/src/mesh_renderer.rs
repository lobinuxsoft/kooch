//! Mesh renderer component.
//!
//! Tags an entity as renderable with traditional polygonal geometry
//! (non-SDF). The render pipeline iterates entities with `MeshRenderer`
//! plus [`Transform`](crate::Transform) to draw meshes.

use ome_core::Guid;

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Component that binds an entity to a mesh and a material for rendering.
///
/// `mesh` references a mesh asset by its persistent [`Guid`] — the same
/// identifier the asset's `.meta` sidecar carries. The meshlet pipeline
/// resolves the GUID to a GPU-resident mesh through the `AssetServer` +
/// `AssetDatabase` resources at sync time. Storing the GUID (not a
/// runtime `Handle<T>`) keeps the component **persistible**: scene
/// serialization round-trips cleanly across runs and the reference
/// stays valid even if the source file is moved (as long as its
/// sidecar follows it).
///
/// `material` is a legacy string and currently unused — the material
/// system migrates to `Assets<Material>` in a follow-up PR. It remains
/// here so scene serialization round-trips today; the typed migration
/// will land alongside the material asset infrastructure.
///
/// # Default
///
/// - `mesh`: `None`
/// - `material`: `""`
/// - `visible`: true
/// - `cast_shadows`: true
/// - `receive_shadows`: true
#[derive(Debug, Clone, Reflect)]
#[reflect(category = "Rendering")]
pub struct MeshRenderer {
    /// When `Some`, the meshlet pipeline picks this entity up via the
    /// scene cull. Persistent across runs — the GUID lives in the
    /// asset's `.meta` sidecar.
    #[reflect(skip)]
    pub mesh: Option<Guid>,
    /// Asset path for the material (legacy `String`, unused — pending
    /// migration to `Assets<Material>` once that asset exists).
    pub material: String,
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
            material: String::new(),
            visible: true,
            cast_shadows: true,
            receive_shadows: true,
        }
    }
}

impl Component for MeshRenderer {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflect::Reflect;

    #[test]
    fn default_values() {
        let r = MeshRenderer::default();
        assert!(r.mesh.is_none());
        assert!(r.material.is_empty());
        assert!(r.visible);
        assert!(r.cast_shadows);
        assert!(r.receive_shadows);
    }

    #[test]
    fn reflect_fields() {
        let r = MeshRenderer::default();
        let fields = r.reflect_fields();
        let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        // `mesh` is `#[reflect(skip)]` — opaque GUID, not editor-
        // inspector friendly until the asset-picker UI lands.
        assert_eq!(
            names,
            &["material", "visible", "cast_shadows", "receive_shadows"]
        );
    }

    #[test]
    fn mesh_field_round_trips_a_guid() {
        let g = Guid::new_v4();
        let r = MeshRenderer {
            mesh: Some(g),
            ..Default::default()
        };
        assert_eq!(r.mesh, Some(g));
    }
}
