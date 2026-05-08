use std::path::PathBuf;
use std::time::SystemTime;

/// Bookkeeping for a single asset registered with the database.
#[derive(Debug, Clone)]
pub struct AssetEntry {
    /// Absolute or project-relative path to the source file. The
    /// database stores whatever the caller registers; convention is
    /// project-relative so paths are portable across machines.
    pub path: PathBuf,
    /// Last-modified time of the source file when registered. Used
    /// (later) by hot-reload to detect external edits.
    pub mtime: SystemTime,
    /// Concrete asset type the loader produced (e.g.
    /// `"ome_render::meshlet::MeshletMesh"`). Mirrors
    /// `AssetMeta::asset_type` from the sidecar. `None` means the
    /// scanner saw the sidecar but no `AssetServer::load::<T>` has
    /// run for the path yet — once it does, the field is filled in
    /// and stays put.
    pub type_name: Option<String>,
}
