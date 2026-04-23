//! Mesh rendering — glTF mesh loading + a render pass that draws every
//! visible `MeshRenderer + GlobalTransform` entity from the ECS.
//!
//! Composes alongside the ray-march pass: the editor's viewport pipeline
//! runs the SDF pass first (clears the offscreen target), then this pass
//! with `LoadOp::Load` so meshes are layered on top of the SDF image.
//!
//! Scope is intentionally small for the MVP (issue #129):
//! - First mesh, first primitive of the glTF document.
//! - Vertex attributes: position, normal, uv0 (uv0 unused by the shader yet).
//! - Materials are ignored — fragment shader emits world-space normal as RGB.
//! - No depth buffer; meshes always paint over the SDF image.

mod gpu_mesh;
mod loader;
mod renderer;

pub use gpu_mesh::{Aabb, GpuMesh, MeshVertex};
pub use loader::{MeshLoadError, MeshLoader};
pub use renderer::MeshPassRenderer;

const SHADER_SOURCE: &str = include_str!("../../shaders/mesh_main.wgsl");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_parses() {
        let module = naga::front::wgsl::parse_str(SHADER_SOURCE)
            .expect("mesh shader should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("mesh shader should validate");
    }
}
