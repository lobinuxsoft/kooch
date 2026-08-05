//! Editor systems — startup and per-frame render.

mod asset_detail;
mod pacing;
mod present;
mod project_assets;
mod remote_sync;
mod render;
mod startup;
mod tab_viewer;

pub(crate) use project_assets::{
    LastScannedProject, ensure_main_exists_system, scan_project_assets_system,
};
pub(crate) use remote_sync::{RemoteSyncState, remote_sync_system};
pub(crate) use render::editor_render_system;
pub(crate) use startup::editor_startup_system;
