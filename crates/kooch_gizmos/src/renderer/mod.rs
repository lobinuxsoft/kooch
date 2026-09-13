//! Gizmo line renderer: queues segments and draws them as screen-space quads.

mod batch;
mod gizmo_renderer;
mod grid_pass;
mod helpers;
mod types;

pub use batch::GizmoBatch;
pub use gizmo_renderer::GizmoRenderer;
pub use grid_pass::{GridPass, GridPlane};
pub use types::{DEFAULT_LINE_THICKNESS, LineSegment};
