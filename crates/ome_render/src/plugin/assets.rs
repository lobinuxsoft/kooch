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

use ome_core::app::App;
use ome_core::asset_database::AssetDatabase;
use ome_core::asset_loader::AssetServer;
use ome_core::assets::Assets;
use ome_core::gpu::GpuContext;
use ome_core::plugin::Plugin;
use ome_core::resource::Resources;
use ome_core::stage::Stage;

use crate::material::{Material, MaterialLoader, MaterialPipeline};
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
        server.register_loader::<Material, _>(MaterialLoader);

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
        app.insert_resource(Assets::<Material>::new());

        // The `MaterialPipeline` needs a `wgpu::Device`, which is
        // not available at plugin-build time. Defer construction to
        // a Stage::Startup system that runs after WindowPlugin
        // inserts the `GpuContext`. The system also re-runs lazily
        // from inside the editor render path if startup ordering
        // ever leaves us without a context.
        app.add_system(Stage::Startup, init_material_pipeline_system);

        // Eager-load every `.glb` / `.gltf` in the asset root as a
        // `MeshletMesh`. Two effects we want at first frame:
        // 1. Sidecars created before PR4 (no `asset_type`) get
        //    back-filled by `read_or_create_typed`, so the database
        //    registers them with the correct type and the inspector
        //    picker can list them.
        // 2. The GPU-side cache (`MeshletRenderStage::sync_assets_to_gpu`)
        //    short-circuits: by the time the user picks an asset the
        //    bytes are already through the loader, with the upload
        //    deferred to whichever entity references the GUID.
        //
        // Mirrors Unity's "every Asset gets imported on project load"
        // contract. Other typed extensions (PNG → Image, etc.) plug
        // in through the same loop as their loaders register; only
        // glb is wired today because nothing else has a typed asset
        // story yet.
        eager_import_typed_assets(app);
    }
}

fn eager_import_typed_assets(app: &mut App) {
    let resources = app.resources_mut();
    eager_import_with(resources);
}

/// Stand-alone version of [`eager_import_typed_assets`] callable
/// against a `&mut Resources` — used by the project-side scan system
/// after a fresh project root is added to the database.
pub(crate) fn eager_import_with(resources: &mut Resources) {
    // Snapshot every registered asset path before we start mutating
    // resources — we need to release the database borrow before
    // calling `AssetServer::load`, which in turn touches the
    // database via `ensure_guid_identity`.
    let scanned: Vec<PathBuf> = resources
        .get::<AssetDatabase>()
        .map(|db| db.path_iter().map(|(p, _)| p.to_path_buf()).collect())
        .unwrap_or_default();

    if scanned.is_empty() {
        return;
    }

    let Some(mut server) = resources.remove::<AssetServer>() else {
        return;
    };

    let mut counts = ImportCounts::default();
    for path in &scanned {
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        let ext_lower = ext.to_ascii_lowercase();
        match ext_lower.as_str() {
            "glb" | "gltf" => {
                if let Err(e) = server.load::<MeshletMesh>(path, resources) {
                    tracing::warn!(
                        target: "ome_render::plugin::assets",
                        path = %path.display(),
                        error = %e,
                        "eager MeshletMesh import failed",
                    );
                } else {
                    counts.meshlet += 1;
                }
            }
            "png" | "jpg" | "jpeg" => {
                if let Err(e) = server.load::<Image>(path, resources) {
                    tracing::warn!(
                        target: "ome_render::plugin::assets",
                        path = %path.display(),
                        error = %e,
                        "eager Image import failed",
                    );
                } else {
                    counts.image += 1;
                }
            }
            _ => {
                // RON files use the *compound* extension
                // `.ome_material.ron`; `Path::extension` only sees
                // the last segment (`ron`). Match on the suffix
                // string instead.
                if path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|s| s.ends_with(".ome_material.ron"))
                {
                    if let Err(e) = server.load::<Material>(path, resources) {
                        tracing::warn!(
                            target: "ome_render::plugin::assets",
                            path = %path.display(),
                            error = %e,
                            "eager Material import failed",
                        );
                    } else {
                        counts.material += 1;
                    }
                }
            }
        }
    }
    resources.insert(server);

    if counts.any() {
        tracing::info!(
            target: "ome_render::plugin::assets",
            meshlet = counts.meshlet,
            image = counts.image,
            material = counts.material,
            "eager-imported typed assets",
        );
    }
}

#[derive(Default)]
struct ImportCounts {
    meshlet: usize,
    image: usize,
    material: usize,
}

impl ImportCounts {
    fn any(&self) -> bool {
        self.meshlet + self.image + self.material > 0
    }
}

fn init_material_pipeline_system(resources: &mut Resources) {
    if resources.get::<MaterialPipeline>().is_some() {
        return;
    }
    let Some(gpu) = resources.get::<GpuContext>() else {
        tracing::warn!(
            target: "ome_render::plugin::assets",
            "GpuContext missing at Startup; MaterialPipeline init deferred",
        );
        return;
    };
    let pipeline = MaterialPipeline::new(gpu.device());
    drop(gpu);
    resources.insert(pipeline);
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
