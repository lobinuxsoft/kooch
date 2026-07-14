//! Editor systems — startup and per-frame render.

mod asset_detail;
mod present;
mod project_assets;
mod render;
mod startup;
mod tab_viewer;

pub(crate) use project_assets::{LastScannedProject, scan_project_assets_system};
pub(crate) use render::editor_render_system;
pub(crate) use startup::editor_startup_system;
