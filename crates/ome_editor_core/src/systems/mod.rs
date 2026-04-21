//! Editor systems — startup and per-frame render.

mod present;
mod render;
mod startup;
mod tab_viewer;

pub(crate) use render::editor_render_system;
pub(crate) use startup::editor_startup_system;
