//! Gizmo render pass — unlit colored lines for editor overlays.
//!
//! Used by the editor to visualize selection bounding boxes, axis lines,
//! and (in future PRs) interactive transform handles. Not used by the
//! play-mode `RenderPlugin` since gizmos are an editor-only concern.
//!
//! Architecture: **immediate-mode**. Each frame the editor populates a
//! [`GizmoBatch`] resource with line segments derived from selection
//! state. The renderer reads the batch, uploads to a dynamic vertex
//! buffer, and draws once with `LineList` topology. No persistent
//! gizmo entities — visuals are pure functions of editor state.
//!
//! Lines render always-on-top: depth comparison `Always`, depth-write
//! disabled. Matches the default Unity / Unreal feel for editor gizmos.

mod gizmos;
pub mod mesh;
mod renderer;
mod visualizer;
mod wireframe;

pub use gizmos::Gizmos;
pub use mesh::{MeshBatch, MeshDraw, MeshGizmoRenderer, MeshVertex};
pub use renderer::{DEFAULT_LINE_THICKNESS, GizmoBatch, GizmoRenderer, LineSegment};
pub use visualizer::{Visualizer, VisualizerRegistry};
pub use wireframe::{MAX_CIRCLE_SEGMENTS, MIN_CIRCLE_SEGMENTS, segments_for};

pub(crate) const SHADER_SOURCE: &str = include_str!("../shaders/gizmo_main.wgsl");
