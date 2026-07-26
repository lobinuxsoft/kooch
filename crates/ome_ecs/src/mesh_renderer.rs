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
    #[reflect(asset = "ome_render::meshlet::asset::MeshletMesh")]
    pub mesh: Option<Guid>,
    /// When `Some`, the meshlet scene system resolves the GUID to a
    /// `MaterialPool` slot via the `MaterialPipeline` resource. The
    /// inspector renders this as a typed dropdown of every
    /// `Material` registered with the asset database. `None` falls
    /// back to the white-diffuse default at slot 0.
    #[reflect(asset = "ome_render::material::asset::Material")]
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
mod tests {
    use super::*;
    use crate::reflect::Reflect;

    #[test]
    fn default_values() {
        let r = MeshRenderer::default();
        assert!(r.mesh.is_none());
        assert!(r.material.is_none());
        assert!(r.visible);
        assert!(r.cast_shadows);
        assert!(r.receive_shadows);
    }

    #[test]
    fn material_field_is_asset_ref_with_canonical_type() {
        use crate::reflect::FieldKind;
        let r = MeshRenderer::default();
        let mat_meta = r
            .reflect_fields()
            .iter()
            .find(|f| f.name == "material")
            .expect("material field reflected");
        assert_eq!(mat_meta.kind, FieldKind::AssetRef);
        assert_eq!(
            mat_meta.asset_type, "ome_render::material::asset::Material",
            "attribute must equal `type_name::<Material>()` so the picker filter matches sidecar entries",
        );
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
            &[
                "mesh",
                "material",
                "visible",
                "cast_shadows",
                "receive_shadows"
            ],
        );
    }

    /// Regression: the `asset_type` string MUST match
    /// `std::any::type_name::<T>()` exactly, because the asset server
    /// uses that as the sidecar's `asset_type` value and the inspector
    /// picker filters by exact-match string. Re-exported paths
    /// (e.g. `ome_render::meshlet::MeshletMesh` vs the canonical
    /// `ome_render::meshlet::asset::MeshletMesh`) are NOT equivalent
    /// from the type-system's point of view.
    ///
    /// We can't reach `MeshletMesh` from `ome_ecs` (would create a
    /// dep cycle), but we can encode the canonical path the macro
    /// must emit and trust the assertion to fire if someone edits
    /// the attribute and breaks the contract.
    #[test]
    fn mesh_asset_type_matches_canonical_type_name() {
        let r = MeshRenderer::default();
        let mesh_meta = r
            .reflect_fields()
            .iter()
            .find(|f| f.name == "mesh")
            .expect("mesh field reflected");
        assert_eq!(
            mesh_meta.asset_type, "ome_render::meshlet::asset::MeshletMesh",
            "attribute string must equal `type_name::<MeshletMesh>()` so the picker filter matches the AssetEntry's type_name",
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
        assert_eq!(
            mesh_meta.asset_type,
            "ome_render::meshlet::asset::MeshletMesh"
        );
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
                asset_type: "ome_render::meshlet::asset::MeshletMesh".to_owned(),
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
                asset_type: "ome_render::meshlet::asset::MeshletMesh".to_owned(),
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
