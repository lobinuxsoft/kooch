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
/// `mesh` is the asset key into `Assets<MeshletMesh>` — the meshlet
/// pipeline picks the entity up via the scene cull when `Some`. The key
/// is opaque to `ome_ecs` (kept as `u64` so this crate stays free of an
/// `ome_render` dependency); `ome_render`'s scene system converts it
/// into a typed `Handle<MeshletMesh>` at lookup time.
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
    /// scene cull. Stored type-erased so `ome_ecs` stays independent of
    /// `ome_render`'s typed handle wrapper.
    #[reflect(skip)]
    pub mesh: Option<MeshletAssetKey>,
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
        // `mesh` is `#[reflect(skip)]` — opaque key, not editor-inspector
        // friendly until the asset-picker UI lands.
        assert_eq!(
            names,
            &["material", "visible", "cast_shadows", "receive_shadows"]
        );
    }
}
