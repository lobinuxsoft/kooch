//! Gizmo renderer: queues line segments and renders them as
//! screen-space quads.
//!
//! Submodules:
//! - [`types`] — public [`LineSegment`] + private GPU types.
//! - [`batch`] — public [`GizmoBatch`] queue API.
//! - [`gizmo_renderer`] — public [`GizmoRenderer`] pipeline.
//! - [`helpers`] — internal vertex emission + camera resolution.

mod batch;
mod gizmo_renderer;
mod helpers;
mod types;

pub use batch::GizmoBatch;
pub use gizmo_renderer::GizmoRenderer;
pub use types::{DEFAULT_LINE_THICKNESS, LineSegment};
