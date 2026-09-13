//! Editor-only gizmos — lines, filled meshes and the grid — drawn in immediate mode from editor
//! state each frame.
//! Handles draw on top (`Always`); the grid depth-tests so the scene occludes it.

mod gizmos;
pub mod grid;
pub mod mesh;
mod renderer;
mod visualizer;
mod wireframe;

pub use gizmos::Gizmos;
pub use mesh::{MeshBatch, MeshDraw, MeshGizmoRenderer, MeshVertex};
pub use renderer::{
    DEFAULT_LINE_THICKNESS, GizmoBatch, GizmoRenderer, GridPass, GridPlane, LineSegment,
};
pub use visualizer::{Visualizer, VisualizerRegistry};
pub use wireframe::{MAX_CIRCLE_SEGMENTS, MIN_CIRCLE_SEGMENTS, segments_for};

pub(crate) const GRID_SHADER: &str = include_str!("../shaders/grid.wgsl");
pub(crate) const SHADER_SOURCE: &str = include_str!("../shaders/gizmo_main.wgsl");
