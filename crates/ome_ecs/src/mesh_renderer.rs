//! Mesh renderer component.
//!
//! Tags an entity as renderable with traditional polygonal geometry
//! (non-SDF). The render pipeline iterates entities with `MeshRenderer`
//! plus [`Transform`](crate::Transform) to draw meshes.

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Component that binds an entity to a mesh and a material for rendering.
///
/// `mesh` and `material` are asset paths (strings) until the Asset Handle
/// system lands (see tracking issue). They will be replaced by typed
/// `Handle<Mesh>` / `Handle<Material>` at that point.
///
/// # Default
///
/// - `mesh`: `""`
/// - `material`: `""`
/// - `visible`: true
/// - `cast_shadows`: true
/// - `receive_shadows`: true
#[derive(Debug, Clone, Reflect)]
pub struct MeshRenderer {
    /// Asset path or handle key for the mesh (e.g. `"meshes/cube.glb"`).
    pub mesh: String,
    /// Asset path or handle key for the material.
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
            mesh: String::new(),
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
        assert_eq!(
            names,
            &["mesh", "material", "visible", "cast_shadows", "receive_shadows"]
        );
    }
}
