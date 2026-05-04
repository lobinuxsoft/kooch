//! [`AssetPlugin`] — wires the engine's asset infrastructure into the
//! `App`'s `Resources`.
//!
//! At plugin build time the following resources are inserted:
//!
//! - [`AssetServer`] with the configured asset root and every loader
//!   registered (Mesh / MeshletMesh / Image — extend here when new
//!   asset types arrive).
//! - [`AssetDatabase`] populated by an initial recursive scan of the
//!   asset root. Sidecar `.meta` files found there register their
//!   GUIDs immediately so scene-side `load_by_guid` lookups work
//!   without an explicit prior `load(path)`.
//! - `Assets<T>` storages for each asset type the loaders produce.
//!
//! The plugin is independent of [`super::RenderPlugin`] — game tools,
//! servers, and headless test harnesses can install asset loading
//! without pulling in the GPU render pipeline.

use std::path::PathBuf;

use ome_core::asset_database::AssetDatabase;
use ome_core::asset_loader::AssetServer;
use ome_core::assets::Assets;
use ome_core::app::App;
use ome_core::plugin::Plugin;

use crate::mesh::{GltfMeshLoader, Mesh};
use crate::meshlet::{MeshletMesh, MeshletMeshLoader};
use crate::texture::{Image, ImageLoader};

/// Plugin that installs the engine-wide asset pipeline.
///
/// Configure the asset root via [`AssetPlugin::with_root`]. Defaults
/// to `assets/` (project-relative).
#[derive(Debug, Clone)]
pub struct AssetPlugin {
    asset_root: PathBuf,
}

impl AssetPlugin {
    /// Constructs the plugin with the default asset root (`assets/`).
    pub fn new() -> Self {
        Self {
            asset_root: PathBuf::from("assets"),
        }
    }

    /// Overrides the asset root. Useful for tests (point at a tempdir)
    /// and for tools that ship their own asset trees.
    pub fn with_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.asset_root = root.into();
        self
    }
}

impl Default for AssetPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for AssetPlugin {
    fn name(&self) -> &str {
        "AssetPlugin"
    }

    fn build(&self, app: &mut App) {
        let mut server = AssetServer::new().with_asset_root(self.asset_root.clone());
        server.register_loader::<Mesh, _>(GltfMeshLoader);
        server.register_loader::<MeshletMesh, _>(MeshletMeshLoader);
        server.register_loader::<Image, _>(ImageLoader::srgb());

        let mut database = AssetDatabase::new();
        match database.scan_directory(&self.asset_root) {
            Ok(report) => {
                tracing::info!(
                    target: "ome_render::plugin::assets",
                    root = %self.asset_root.display(),
                    registered = report.registered,
                    orphans = report.orphans,
                    duplicates = report.duplicates,
                    "asset database initial scan complete",
                );
            }
            Err(e) => {
                tracing::warn!(
                    target: "ome_render::plugin::assets",
                    root = %self.asset_root.display(),
                    error = %e,
                    "asset database initial scan failed; continuing with empty registry"
                );
            }
        }

        app.insert_resource(server);
        app.insert_resource(database);
        app.insert_resource(Assets::<Mesh>::new());
        app.insert_resource(Assets::<MeshletMesh>::new());
        app.insert_resource(Assets::<Image>::new());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ome_core::asset_meta::{write_meta, AssetMeta};
    use std::fs;
    use std::io::Write;

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new(name: &str) -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!("ome_asset_plugin_{name}_{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn touch(path: &std::path::Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        let mut f = fs::File::create(path).expect("create source");
        f.write_all(b"placeholder").expect("write");
    }

    #[test]
    fn build_inserts_all_expected_resources() {
        let dir = TempDir::new("resources");
        let mut app = App::new();
        AssetPlugin::new().with_root(&dir.path).build(&mut app);

        let resources = app.resources();
        assert!(resources.get::<AssetServer>().is_some());
        assert!(resources.get::<AssetDatabase>().is_some());
        assert!(resources.get::<Assets<Mesh>>().is_some());
        assert!(resources.get::<Assets<MeshletMesh>>().is_some());
        assert!(resources.get::<Assets<Image>>().is_some());
    }

    #[test]
    fn registered_loaders_cover_glb_png() {
        let dir = TempDir::new("loaders");
        let mut app = App::new();
        AssetPlugin::new().with_root(&dir.path).build(&mut app);

        let server = app.resources().get::<AssetServer>().unwrap();
        assert!(server.has_loader::<Mesh>());
        assert!(server.has_loader::<MeshletMesh>());
        assert!(server.has_loader::<Image>());

        let mesh_exts = server.extensions_for::<Mesh>();
        assert!(mesh_exts.contains(&"glb"));

        let meshlet_exts = server.extensions_for::<MeshletMesh>();
        assert!(meshlet_exts.contains(&"glb"));

        let image_exts = server.extensions_for::<Image>();
        assert!(image_exts.contains(&"png"));
    }

    #[test]
    fn initial_scan_picks_up_existing_meta_files() {
        let dir = TempDir::new("scan");
        let asset = dir.path.join("meshes/foo.glb");
        touch(&asset);
        let meta = AssetMeta::new();
        let expected = meta.guid;
        write_meta(&asset, &meta).expect("write meta");

        let mut app = App::new();
        AssetPlugin::new().with_root(&dir.path).build(&mut app);

        let db = app.resources().get::<AssetDatabase>().unwrap();
        assert_eq!(db.len(), 1, "scan should have registered the seeded asset");
        assert_eq!(db.guid_for(&asset), Some(expected));
    }

    #[test]
    fn missing_root_directory_does_not_panic() {
        let mut app = App::new();
        AssetPlugin::new()
            .with_root("/nonexistent/path/xyz")
            .build(&mut app);

        let db = app.resources().get::<AssetDatabase>().unwrap();
        assert_eq!(db.len(), 0);
    }
}
