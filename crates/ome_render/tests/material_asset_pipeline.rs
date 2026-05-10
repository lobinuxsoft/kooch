//! End-to-end test for the material asset pipeline.
//!
//! Walks the same chain the editor exercises when the inspector
//! picker assigns a `.ron` material to `MeshRenderer.material`:
//! tempdir-on-disk `.ron` + `.meta` → `AssetDatabase` → `AssetServer`
//! → `MaterialPipeline::sync_from_resources` → `register` →
//! `lookup_or_fallback` returns a non-fallback slot.
//!
//! Differs from `meshlet_materials.rs` (which calls
//! `MaterialPipeline.register` directly with a freshly-minted GUID
//! and bypasses the asset pipeline entirely) by going through
//! `AssetServer::load_by_guid`. Regression guard for #533: the bug
//! that surfaced there was invisible to the bypassed path, so this
//! test must own the picker → GPU contract.
//!
//! Headless except for the wgpu device the pool needs — gated on
//! `try_acquire_device`.

mod common;

use std::path::{Path, PathBuf};

use ome_core::Guid;
use ome_core::asset_database::{AssetDatabase, AssetEntry};
use ome_core::asset_loader::AssetServer;
use ome_core::asset_meta::{write_meta, AssetMeta};
use ome_core::assets::Assets;
use ome_core::resource::Resources;
use ome_ecs::EntityAllocator;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::component::registry::ComponentRegistry;
use ome_ecs::query::AccessTracker;
use ome_render::material::{
    FALLBACK_MATERIAL_ID, MATERIAL_TYPE_NAME, Material, MaterialLoader, MaterialPipeline,
};
use ome_render::meshlet::{MeshletRenderStage, MeshletRenderStageConfig};

use common::try_acquire_device;

struct TempDir {
    path: PathBuf,
}
impl TempDir {
    fn new(name: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "ome_material_asset_{name}_{}",
            std::process::id(),
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn write_material_asset(dir: &Path, name: &str, ron_body: &str) -> (PathBuf, Guid) {
    let path = dir.join(format!("{name}.ron"));
    std::fs::write(&path, ron_body).expect("write material ron");
    let mut meta = AssetMeta::new();
    meta.asset_type = Some(MATERIAL_TYPE_NAME.to_owned());
    let guid = meta.guid;
    write_meta(&path, &meta).expect("write material meta");
    (path, guid)
}

fn ron_blue_metal() -> &'static str {
    "(\n    base_color: (0.10, 0.30, 0.85, 1.0),\n    metallic: 0.85,\n    roughness: 0.20,\n    emissive: 0.0,\n)\n"
}

fn build_resources(database: AssetDatabase, server: AssetServer) -> Resources {
    let mut resources = Resources::new();
    // ECS scaffolding the meshlet stage queries through when it
    // collects referenced GUIDs. Even the material-only tests need
    // this when `MeshletRenderStage::sync_assets_to_gpu` runs end-
    // to-end, so we install it unconditionally for shape parity.
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(AccessTracker::new());
    resources.insert(server);
    resources.insert(database);
    resources.insert(Assets::<Material>::new());
    resources
}

fn registered_database(asset_path: &Path, guid: Guid) -> AssetDatabase {
    let mut db = AssetDatabase::new();
    db.register(
        guid,
        AssetEntry {
            path: asset_path.to_path_buf(),
            mtime: std::time::SystemTime::now(),
            type_name: Some(MATERIAL_TYPE_NAME.to_owned()),
        },
    );
    db
}

fn loader_ready_server() -> AssetServer {
    let mut server = AssetServer::new();
    server.register_loader::<Material, _>(MaterialLoader);
    server
}

#[test]
fn sync_from_resources_registers_material_from_disk() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };

    let dir = TempDir::new("register");
    let (asset_path, guid) = write_material_asset(&dir.path, "blue_metal", ron_blue_metal());

    let database = registered_database(&asset_path, guid);
    let server = loader_ready_server();
    let mut resources = build_resources(database, server);

    let mut pipeline = MaterialPipeline::new(&device);
    assert_eq!(pipeline.registered_count(), 0);
    assert_eq!(
        pipeline.lookup_or_fallback(Some(guid)),
        FALLBACK_MATERIAL_ID,
        "before sync, the picker GUID must miss and fall back",
    );

    pipeline.sync_from_resources(&queue, &mut resources);

    assert_eq!(
        pipeline.registered_count(),
        1,
        "sync must register exactly the one material the database carries",
    );
    let slot = pipeline.lookup(guid).expect("blue_metal GUID is now known");
    assert_ne!(
        slot, FALLBACK_MATERIAL_ID,
        "registered slot must not collide with the fallback white-diffuse",
    );
    assert_eq!(
        pipeline.lookup_or_fallback(Some(guid)),
        slot,
        "lookup_or_fallback must agree with lookup post-sync",
    );
}

#[test]
fn sync_from_resources_is_idempotent_across_frames() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };

    let dir = TempDir::new("idempotent");
    let (asset_path, guid) = write_material_asset(&dir.path, "blue_metal", ron_blue_metal());

    let database = registered_database(&asset_path, guid);
    let server = loader_ready_server();
    let mut resources = build_resources(database, server);

    let mut pipeline = MaterialPipeline::new(&device);
    pipeline.sync_from_resources(&queue, &mut resources);
    let first_slot = pipeline.lookup(guid).expect("registered on first sync");

    // Subsequent syncs must reuse the same slot — the editor calls
    // `sync_from_resources` every frame, so any per-frame slot churn
    // would explode the pool on long sessions.
    pipeline.sync_from_resources(&queue, &mut resources);
    pipeline.sync_from_resources(&queue, &mut resources);

    assert_eq!(pipeline.registered_count(), 1, "no slot churn across frames");
    assert_eq!(
        pipeline.lookup(guid),
        Some(first_slot),
        "GUID must reuse its first-frame slot",
    );
}

#[test]
fn editor_path_syncs_material_with_gpu_context_outside_resources() {
    // Regression guard for #533. The editor's frame-driver removes
    // `GpuContext` from `Resources` for the whole frame, so by the
    // time `MeshletRenderStage::sync_assets_to_gpu` ran the previous
    // implementation's inner `resources.remove::<GpuContext>()`
    // returned `None` and `MaterialPipeline::sync_from_resources`
    // was silently skipped. This test mirrors the editor's call
    // shape — queue + device come in as parameters and `Resources`
    // never holds a `GpuContext` — and asserts the picker GUID ends
    // up registered after a single tick.
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };

    let dir = TempDir::new("editor_path");
    let (asset_path, material_guid) =
        write_material_asset(&dir.path, "blue_metal", ron_blue_metal());

    let database = registered_database(&asset_path, material_guid);
    let server = loader_ready_server();
    let mut resources = build_resources(database, server);

    resources.insert(MaterialPipeline::new(&device));

    let mut stage = MeshletRenderStage::new(
        &device,
        MeshletRenderStageConfig {
            size: (64, 64),
            instance_capacity: 4,
            ..Default::default()
        },
    );

    stage.sync_assets_to_gpu(&device, &queue, &mut resources);

    let pipeline = resources
        .get::<MaterialPipeline>()
        .expect("MaterialPipeline must be reinserted after sync_assets_to_gpu");
    let slot = pipeline
        .lookup(material_guid)
        .expect("editor-path tick must register the material under its picker GUID");
    assert_ne!(
        slot, FALLBACK_MATERIAL_ID,
        "registered slot must not collide with the fallback white-diffuse",
    );
}

#[test]
fn sync_from_resources_skips_database_without_material_entries() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };

    let dir = TempDir::new("empty_db");
    let (asset_path, guid) = write_material_asset(&dir.path, "blue_metal", ron_blue_metal());

    // Database is populated, but with a non-material `type_name`. The
    // sync must walk past it silently — exercising the
    // `entries_of_type(MATERIAL_TYPE_NAME)` filter that gates the
    // entire pass.
    let mut database = AssetDatabase::new();
    database.register(
        guid,
        AssetEntry {
            path: asset_path,
            mtime: std::time::SystemTime::now(),
            type_name: Some("ome_render::meshlet::MeshletMesh".to_owned()),
        },
    );
    let server = loader_ready_server();
    let mut resources = build_resources(database, server);

    let mut pipeline = MaterialPipeline::new(&device);
    pipeline.sync_from_resources(&queue, &mut resources);

    assert_eq!(
        pipeline.registered_count(),
        0,
        "no material entries → no registrations",
    );
    assert_eq!(
        pipeline.lookup_or_fallback(Some(guid)),
        FALLBACK_MATERIAL_ID,
        "GUID hidden behind a non-material type_name stays in fallback",
    );
}
