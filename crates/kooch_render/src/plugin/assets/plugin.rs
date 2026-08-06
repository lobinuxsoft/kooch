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
#[derive(Clone)]
pub struct AssetPlugin {
    roots: Vec<PathBuf>,
    /// Whether to pull every typed asset into memory at build time.
    eager_import: bool,
    /// Loaders contributed by crates this one does not depend on.
    ///
    /// The server is built here, and a plugin reaching into a resource
    /// another plugin may not have inserted yet is an ordering bug — the
    /// reason prefabs register through a free function called from this
    /// build. That works because `kooch_render` depends on `kooch_ecs`.
    ///
    /// It does not work for `kooch_input`, and would not for audio, so
    /// the alternative was to make the renderer depend on both. Whoever
    /// assembles the app knows which asset types exist; the renderer does
    /// not need to.
    extra_loaders: Vec<std::sync::Arc<dyn Fn(&mut AssetServer) + Send + Sync>>,
    /// The `Assets<T>` those loaders fill. Paired with `extra_loaders` by
    /// `with_asset`, which is the only thing that pushes to either.
    extra_storages: Vec<std::sync::Arc<dyn Fn(&mut App) + Send + Sync>>,
}

impl std::fmt::Debug for AssetPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssetPlugin")
            .field("roots", &self.roots)
            .field("eager_import", &self.eager_import)
            .field("extra_loaders", &self.extra_loaders.len())
            .field("extra_storages", &self.extra_storages.len())
            .finish()
    }
}

impl AssetPlugin {
    /// Constructs the plugin with the default asset root (`assets/`,
    /// relative to the working directory).
    pub fn new() -> Self {
        Self {
            roots: vec![PathBuf::from("assets")],
            eager_import: true,
            extra_loaders: Vec::new(),
            extra_storages: Vec::new(),
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

    /// Adds an asset type from a crate this one does not know about:
    /// its loader **and** the storage that loader fills.
    ///
    /// ```ignore
    /// AssetPlugin::new().with_asset::<ActionMap, _>(InputMapLoader)
    /// ```
    ///
    /// The loader is registered while the server is being built, so it is
    /// in place before the first scan — which is what a loader registered
    /// from a `Startup` system would miss.
    ///
    /// # Why both, and not a loader alone
    ///
    /// `load_by_guid` requires `Assets<T>` to already exist rather than
    /// creating it, so a loader without its storage fails every load with
    /// `Assets<T> resource missing`. That has now happened twice —
    /// `SceneDocument` (see its note below) and `ActionMap` — because the
    /// two were registered in two places and only one of them was a list
    /// a contributor could add to. Taking the type parameter means the
    /// storage cannot be forgotten: there is nowhere to forget it.
    pub fn with_asset<T, L>(mut self, loader: L) -> Self
    where
        T: kooch_core::assets::Asset,
        L: kooch_core::asset_loader::AssetLoader<T> + Clone + Send + Sync + 'static,
    {
        self.extra_loaders.push(std::sync::Arc::new(move |server| {
            server.register_loader::<T, _>(loader.clone());
        }));
        self.extra_storages
            .push(std::sync::Arc::new(|app: &mut App| {
                app.insert_resource(Assets::<T>::new());
            }));
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
        // Every asset type linked into this binary, declared next to
        // itself with `register_asset!`. Nothing lists them, so nothing
        // can leave one out — the failure that shipped `.inputmap` with
        // its loader in two places and its storage in neither.
        for registration in kooch_core::asset_registry::registered_asset_types() {
            (registration.register_loader)(&mut server);
        }
        // Before the scan below, so a contributed type is loadable on the
        // first pass rather than on the second.
        for register in &self.extra_loaders {
            register(&mut server);
        }

        let mut database = AssetDatabase::new();
        // Derived from the loaders just registered above, so a file
        // written by hand is adopted on the first scan rather than being
        // invisible until something loads it.
        let known = server.known_extensions();
        for root in &self.roots {
            match database.scan_directory_adopting(root, &known) {
                Ok(report) => {
                    tracing::info!(
                        target: "kooch_render::plugin::assets",
                        root = %root.display(),
                        registered = report.registered,
                        adopted = report.adopted,
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
        // The storage half of every declared type, and of every
        // `with_asset`. A loader without one fails every load with
        // `Assets<T> resource missing`, so neither path can skip it.
        for registration in kooch_core::asset_registry::registered_asset_types() {
            (registration.install_storage)(app.resources_mut());
        }
        for install in &self.extra_storages {
            install(app);
        }

        // The `MaterialPipeline` needs a `wgpu::Device`, which is
        // not available at plugin-build time. Defer construction to
        // a Stage::Startup system that runs after WindowPlugin
        // inserts the `GpuContext`. The system also re-runs lazily
        // from inside the editor render path if startup ordering
        // ever leaves us without a context.
        app.add_system(Stage::Startup, init_material_pipeline_system);
        // Publishes the project's RenderSettings into the Resources the
        // shading model reads (#744). Per frame, because the asset is
        // reloaded in place when saved and there is no change signal to
        // subscribe to; it returns early unless a value actually moved.
        app.add_system(Stage::Update, crate::settings::apply_render_settings_system);

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

#[cfg(test)]
mod tests {
    use super::*;
    use kooch_core::asset_loader::{AssetLoader, AssetResult, LoadContext};

    #[derive(Debug, Clone, PartialEq)]
    struct Probe(String);

    #[derive(Clone)]
    struct ProbeLoader;
    impl AssetLoader<Probe> for ProbeLoader {
        fn extensions(&self) -> &[&'static str] {
            &["probe"]
        }
        fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<Probe> {
            Ok(Probe(String::from_utf8_lossy(&bytes).into_owned()))
        }
    }

    fn empty_plugin() -> AssetPlugin {
        AssetPlugin::new().with_root(std::env::temp_dir().join("kooch_no_such_assets"))
    }

    /// 🔴 A contributed asset arrives with **both** halves: the loader
    /// that reads it and the `Assets<T>` that loader fills.
    ///
    /// Splitting them is what broke a real run — `load_by_guid` requires
    /// the storage to exist rather than creating it, so an `.inputmap`
    /// with a registered loader failed every frame with `Assets<ActionMap>
    /// resource missing`. Registering a loader alone is no longer
    /// expressible, and this is what says so.
    #[test]
    fn a_contributed_asset_brings_its_loader_and_its_storage() {
        let mut app = App::new();
        empty_plugin()
            .with_asset::<Probe, _>(ProbeLoader)
            .build(&mut app);

        let resources = app.resources_mut();
        assert!(
            resources
                .get::<AssetServer>()
                .is_some_and(|server| server.has_loader::<Probe>()),
            "the contributed loader never reached the server"
        );
        assert!(
            resources.get::<Assets<Probe>>().is_some(),
            "the loader is registered with nowhere to put what it loads,              which fails every load with `Assets<T> resource missing`"
        );
    }

    /// Several crates contributing is the case this exists for — input
    /// today, audio next — so more than one has to survive.
    #[test]
    fn every_contributed_asset_survives() {
        #[derive(Debug, Clone)]
        struct Other(u8);
        #[derive(Clone)]
        struct OtherLoader;
        impl AssetLoader<Other> for OtherLoader {
            fn extensions(&self) -> &[&'static str] {
                &["other"]
            }
            fn load(&self, _b: &[u8], _c: &mut LoadContext<'_>) -> AssetResult<Other> {
                Ok(Other(0))
            }
        }

        let mut app = App::new();
        empty_plugin()
            .with_asset::<Probe, _>(ProbeLoader)
            .with_asset::<Other, _>(OtherLoader)
            .build(&mut app);

        let resources = app.resources_mut();
        assert!(resources.get::<Assets<Probe>>().is_some());
        assert!(resources.get::<Assets<Other>>().is_some());
    }
}
