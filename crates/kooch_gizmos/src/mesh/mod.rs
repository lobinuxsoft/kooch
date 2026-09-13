//! Mesh gizmos: alpha-blended triangles for filled visuals — plane handles, rotate tori, boxes.
//! Drawn after the line pass with depth test `Always` and no depth write, so they sit on top of the
//! scene.

mod batch;
mod renderer;

pub use batch::{MeshBatch, MeshDraw, MeshVertex};
pub use renderer::MeshGizmoRenderer;

pub(crate) const SHADER_SOURCE: &str = include_str!("../../shaders/gizmo_mesh.wgsl");
