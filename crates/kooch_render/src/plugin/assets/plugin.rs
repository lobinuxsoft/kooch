use std::path::{Path, PathBuf};

use kooch_core::app::App;
use kooch_core::asset_database::AssetDatabase;
use kooch_core::asset_loader::AssetServer;
use kooch_core::assets::Assets;
use kooch_core::gpu::GpuContext;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::stage::Stage;

use crate::material::{Material, MaterialLoader, MaterialPipeline};
use crate::mesh::{GltfMeshLoader, Mesh};
use crate::meshlet::{MeshletMesh, MeshletMeshLoader};
use crate::texture::{Image, ImageLoader};

use super::eager::eager_import_typed_assets;

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
    /// Whether to pull every typed asset into memory at build time.
    eager_import: bool,
}

impl AssetPlugin {
    /// Constructs the plugin with the default asset root (`assets/`,
    /// relative to the working directory).
    pub fn new() -> Self {
        Self {
            roots: vec![PathBuf::from("assets")],
            eager_import: true,
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

    /// Registers identity and loaders but decodes nothing up front.
    ///
    /// For a host that has to *resolve* assets without drawing any. The
    /// remote authoring host is one: a prefab instance in a scene is a
    /// reference now, so loading a scene means looking a guid up — but
    /// decoding every texture and mesh for a process that never renders
    /// is work with no result.
    ///
    /// Anything actually asked for still loads on demand; this only skips
    /// the pass that pulls in everything ahead of time.
    pub fn headless(mut self) -> Self {
        self.eager_import = false;
        self
    }

    fn primary_root(&self) -> &Path {
        self.roots
            .first()
            .map(|p| p.as_path())
            .unwrap_or_else(|| Path::new("assets"))
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
        // Registered here rather than by `EcsPlugin`, which owns the type:
        // the server is built in this function, and a plugin reaching for a
        // resource another plugin may not have inserted yet is an ordering
        // bug waiting to happen. This is what gives a `.prefab` a guid, so a
        // component field can reference one.
        kooch_ecs::scene::prefab::register_loader(&mut server);

        let mut database = AssetDatabase::new();
        for root in &self.roots {
            match database.scan_directory(root) {
                Ok(report) => {
                    tracing::info!(
                        target: "kooch_render::plugin::assets",
                        root = %root.display(),
                        registered = report.registered,
                        orphans = report.orphans,
                        duplicates = report.duplicates,
                        "asset database scan complete",
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        target: "kooch_render::plugin::assets",
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
        // The store the prefab loader fills, and the cache `spawn_prefab`
        // reads. `load_by_guid` requires it to exist rather than creating
        // it, so without this every prefab load failed with
        // `MissingAssetStorage` and the Inspector sat on "Loading asset…"
        // forever.
        app.insert_resource(Assets::<kooch_ecs::scene::SceneDocument>::new());

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
            if self.eager_import {
                eager_import_typed_assets(app, root);
            }
        }
    }
}

fn init_material_pipeline_system(resources: &mut Resources) {
    if resources.get::<MaterialPipeline>().is_some() {
        tracing::debug!(
            target: "kooch_render::plugin::assets",
            "init_material_pipeline_system: pipeline already present",
        );
        return;
    }
    let Some(gpu) = resources.get::<GpuContext>() else {
        tracing::warn!(
            target: "kooch_render::plugin::assets",
            "GpuContext missing at Startup; MaterialPipeline init deferred",
        );
        return;
    };
    let pipeline = MaterialPipeline::new(gpu.device(), gpu.queue());
    let _ = gpu;
    resources.insert(pipeline);
    tracing::info!(
        target: "kooch_render::plugin::assets",
        "init_material_pipeline_system: MaterialPipeline inserted into Resources",
    );
}
