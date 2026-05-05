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
    /// asset's `.meta` sidecar. The inspector renders this as a typed
    /// dropdown picker that lists every `MeshletMesh` the
    /// `AssetDatabase` has registered.
    #[reflect(asset = "ome_render::meshlet::MeshletMesh")]
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
        // `mesh` is reflected as a typed AssetRef — the inspector
        // renders it via the asset-picker dropdown.
        assert_eq!(
            names,
            &["mesh", "material", "visible", "cast_shadows", "receive_shadows"],
        );
    }

    #[test]
    fn mesh_field_is_asset_ref_with_meshlet_type() {
        use crate::reflect::FieldKind;
        let r = MeshRenderer::default();
        let mesh_meta = r
            .reflect_fields()
            .iter()
            .find(|f| f.name == "mesh")
            .expect("mesh field should be reflected");
        assert_eq!(mesh_meta.kind, FieldKind::AssetRef);
        assert_eq!(mesh_meta.asset_type, "ome_render::meshlet::MeshletMesh");
    }

    #[test]
    fn reflect_get_and_set_round_trip_guid() {
        use crate::reflect::{Reflect, ReflectValue};
        let mut r = MeshRenderer::default();
        let g = Guid::new_v4();

        // Set via reflection.
        r.reflect_set(
            "mesh",
            ReflectValue::AssetRef {
                guid: Some(g),
                asset_type: "ome_render::meshlet::MeshletMesh".to_owned(),
            },
        )
        .expect("set should succeed");
        assert_eq!(r.mesh, Some(g));

        // Get via reflection.
        let got = r.reflect_get("mesh").expect("get should succeed");
        assert_eq!(
            got,
            ReflectValue::AssetRef {
                guid: Some(g),
                asset_type: "ome_render::meshlet::MeshletMesh".to_owned(),
            },
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
