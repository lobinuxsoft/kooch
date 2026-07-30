use std::fs;
use std::io::Write;
use std::path::PathBuf;

use ome_core::app::App;
use ome_core::asset_database::AssetDatabase;
use ome_core::asset_loader::AssetServer;
use ome_core::asset_meta::{AssetMeta, write_meta};
use ome_core::assets::Assets;
use ome_core::plugin::Plugin;

use crate::material::Material;
use crate::mesh::Mesh;
use crate::meshlet::MeshletMesh;
use crate::texture::Image;

use super::plugin::AssetPlugin;

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
    let _ = touch; // silence unused if we add no touched-file tests
    let _ = std::any::TypeId::of::<Material>();
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
