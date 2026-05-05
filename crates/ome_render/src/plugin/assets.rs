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

use std::path::{Path, PathBuf};

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
/// `roots` is the list of directories that get scanned + eager-
/// imported at plugin build time. The first entry is the
/// `AssetServer`'s primary `asset_root` (relative paths the user
/// passes to `load(path)` resolve against it); additional entries
/// only contribute to scan + eager-import. Typical layout:
///
/// - **Editor**: a single root pointing at `<engine>/assets`. The
///   project's own `assets/` is mirrored later by
///   `scan_project_assets_system` once the user opens a project.
/// - **Game runtime / Play mode**: two roots — `<engine>/assets`
///   first (primary), then `<project>/assets` (secondary). Both
///   get scanned at startup so the runtime can resolve every GUID
///   the scene references.
#[derive(Debug, Clone)]
pub struct AssetPlugin {
    roots: Vec<PathBuf>,
}

impl AssetPlugin {
    /// Constructs the plugin with the default asset root (`assets/`,
    /// relative to the working directory).
    pub fn new() -> Self {
        Self {
            roots: vec![PathBuf::from("assets")],
        }
    }

    /// Replaces the primary asset root with `root` and clears any
    /// extras. Equivalent to constructing the plugin from scratch.
    pub fn with_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.roots = vec![root.into()];
        self
    }

    /// Appends another directory the plugin should scan + eager-
    /// import without overriding the primary `asset_root`. Used to
    /// stack the project's `assets/` on top of the engine's at game
    /// runtime so the binary sees both at first frame.
    pub fn with_extra_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.roots.push(root.into());
        self
    }

    fn primary_root(&self) -> &Path {
        self.roots.first().map(|p| p.as_path()).unwrap_or_else(|| Path::new("assets"))
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
        let mut server = AssetServer::new().with_asset_root(self.primary_root().to_path_buf());
        server.register_loader::<Mesh, _>(GltfMeshLoader);
        server.register_loader::<MeshletMesh, _>(MeshletMeshLoader);
        server.register_loader::<Image, _>(ImageLoader::srgb());
        server.register_loader::<Material, _>(MaterialLoader);

        let mut database = AssetDatabase::new();
        for root in &self.roots {
            match database.scan_directory(root) {
                Ok(report) => {
                    tracing::info!(
                        target: "ome_render::plugin::assets",
                        root = %root.display(),
                        registered = report.registered,
                        orphans = report.orphans,
                        duplicates = report.duplicates,
                        "asset database scan complete",
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        target: "ome_render::plugin::assets",
                        root = %root.display(),
                        error = %e,
                        "asset database scan failed; continuing",
                    );
                }
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

        let roots = self.roots.clone();

        // Eager-load every typed file in each configured root. Two effects we want at first frame:
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
        for root in &roots {
            eager_import_typed_assets(app, root);
        }
    }
}

fn eager_import_typed_assets(app: &mut App, root: &Path) {
    let resources = app.resources_mut();
    eager_import_with(resources, root);
}

/// Walks `root` recursively and loads every file with a recognised
/// typed extension through the `AssetServer`. The load step generates
/// `.meta` sidecars on the fly for assets that do not yet have one,
/// back-fills `asset_type` on legacy sidecars, and registers the
/// entry in the `AssetDatabase` — exactly what the inspector picker
/// needs to surface a new asset at first frame.
///
/// Public so the project-side scan system can rerun the same import
/// pass after a project opens.
pub fn eager_import_with(resources: &mut Resources, root: &Path) {
    let scanned = collect_typed_files(root);
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
            "ron" => {
                // PR5 invariant: every `.ron` under `assets/` is a
                // Material. When other RON-authored asset types
                // arrive, this branch grows a discriminator that
                // peeks the nominal struct tag at the head of the
                // file before dispatching to the matching loader.
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
            _ => {}
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

/// Recursive filesystem walk that collects every file under `root`
/// with a known typed extension. Stays alongside the importer
/// because both share the extension allowlist.
fn collect_typed_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_collect(root, &mut out);
    out
}

fn walk_collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            walk_collect(&path, out);
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        // Skip the sidecar files themselves — only consider their
        // source assets.
        if path.extension().is_some_and(|e| e == "meta") {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        let typed = lower.ends_with(".glb")
            || lower.ends_with(".gltf")
            || lower.ends_with(".png")
            || lower.ends_with(".jpg")
            || lower.ends_with(".jpeg")
            || lower.ends_with(".ron");
        if typed {
            out.push(path);
        }
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
