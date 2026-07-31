//! 3D mesh gizmo path — alpha-blended triangle primitives for filled
//! gizmo visuals (translucent plane handles, future rotate tori, custom
//! 3D shapes).
//!
//! Parallel to the line gizmo path (sibling crate-level module). Lines
//! use [`crate::GizmoBatch`] + [`crate::GizmoRenderer`] with the
//! quad-line rendering technique; meshes use [`MeshBatch`] +
//! [`MeshGizmoRenderer`] with a vanilla `TriangleList` pipeline.
//!
//! Both render passes are dispatched by the editor in sequence
//! (lines → meshes), depth-test `Always` and depth-write off, so all
//! gizmos sit visibly on top of world geometry.

mod batch;
mod renderer;

pub use batch::{MeshBatch, MeshDraw, MeshVertex};
pub use renderer::MeshGizmoRenderer;

pub(crate) const SHADER_SOURCE: &str = include_str!("../../shaders/gizmo_mesh.wgsl");
