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

mod renderer;

pub use renderer::{GizmoBatch, GizmoRenderer, LineSegment};

pub(crate) const SHADER_SOURCE: &str = include_str!("../shaders/gizmo_main.wgsl");
