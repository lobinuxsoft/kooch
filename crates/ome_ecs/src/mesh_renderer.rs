//! Mesh renderer component.
//!
//! Tags an entity as renderable with traditional polygonal geometry
//! (non-SDF). The render pipeline iterates entities with `MeshRenderer`
//! plus [`Transform`](crate::Transform) to draw meshes.

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Type-erased asset key used to point a `MeshRenderer` at a meshlet
/// mesh entry in `Assets<MeshletMesh>` without `ome_ecs` having to
/// import `ome_render` or `slotmap`. The raw u64 round-trips through
/// `slotmap::KeyData::as_ffi()` / `from_ffi()` — `ome_render`'s scene
/// system converts back into a typed `Handle<MeshletMesh>` at lookup
/// time.
pub type MeshletAssetKey = u64;

/// Component that binds an entity to a mesh and a material for rendering.
///
/// `mesh` and `material` are legacy asset paths (strings); the
/// `meshlet_mesh` field is the post-Phase-1.E migration target — set
/// it to a `MeshletAssetKey` (extracted from a `Handle<MeshletMesh>`)
/// and the meshlet pipeline picks the entity up.
///
/// Both code paths coexist during the migration so existing entities
/// keep rendering through the legacy `MeshPassRenderer` until they
/// move over.
///
/// # Default
///
/// - `mesh`: `""`
/// - `material`: `""`
/// - `meshlet_mesh`: `None`
/// - `visible`: true
/// - `cast_shadows`: true
/// - `receive_shadows`: true
#[derive(Debug, Clone, Reflect)]
#[reflect(category = "Rendering")]
pub struct MeshRenderer {
    /// Asset path or handle key for the legacy mesh path.
    pub mesh: String,
    /// Asset path or handle key for the material (legacy).
    pub material: String,
    /// When `Some`, the meshlet pipeline picks this entity up via the
    /// Phase 1.E scene cull. Stored type-erased so `ome_ecs` stays
    /// independent of `ome_render`'s typed handle wrapper.
    #[reflect(skip)]
    pub meshlet_mesh: Option<MeshletAssetKey>,
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
            mesh: String::new(),
            material: String::new(),
            meshlet_mesh: None,
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
        assert!(r.mesh.is_empty());
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
        // `meshlet_mesh` is `#[reflect(skip)]` — it's an opaque key, not
        // editor-inspector friendly. Reflect surface stays the same as
        // pre-1.E to keep scene-serialization compatibility.
        assert_eq!(
            names,
            &["mesh", "material", "visible", "cast_shadows", "receive_shadows"]
        );
    }

    #[test]
    fn meshlet_mesh_defaults_to_none() {
        let r = MeshRenderer::default();
        assert!(r.meshlet_mesh.is_none());
    }
}
