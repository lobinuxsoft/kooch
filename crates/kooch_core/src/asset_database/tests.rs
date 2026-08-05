use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::asset_meta::{AssetMeta, write_meta};
use crate::guid::Guid;

use super::database::AssetDatabase;
use super::entry::AssetEntry;
use super::report::ScanReport;

struct TempDir {
    path: PathBuf,
}
impl TempDir {
    fn new(name: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!("kooch_asset_db_{name}_{}", std::process::id(),));
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

fn touch(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    let mut f = fs::File::create(path).expect("create source");
    f.write_all(b"placeholder").expect("write");
}

#[test]
fn empty_dir_scans_to_zero() {
    let dir = TempDir::new("empty");
    let mut db = AssetDatabase::new();
    let report = db.scan_directory(&dir.path).expect("scan");
    assert_eq!(report, ScanReport::default());
    assert!(db.is_empty());
}

#[test]
fn missing_dir_is_no_op() {
    let mut db = AssetDatabase::new();
    let report = db
        .scan_directory(Path::new("/nonexistent/path/xyz"))
        .expect("scan tolerates missing");
    assert_eq!(report.registered, 0);
}

#[test]
fn asset_with_sidecar_is_registered() {
    let dir = TempDir::new("with_sidecar");
    let asset = dir.path.join("foo.glb");
    touch(&asset);
    let meta = AssetMeta::new();
    let expected_guid = meta.guid;
    write_meta(&asset, &meta).expect("write meta");

    let mut db = AssetDatabase::new();
    let report = db.scan_directory(&dir.path).expect("scan");
    assert_eq!(report.registered, 1);
    assert_eq!(report.orphans, 0);
    assert_eq!(db.len(), 1);

    let entry = db.entry(expected_guid).expect("guid registered");
    assert_eq!(entry.path, asset);
    assert_eq!(db.guid_for(&asset), Some(expected_guid));
}

#[test]
fn asset_without_sidecar_is_skipped() {
    let dir = TempDir::new("no_sidecar");
    let asset = dir.path.join("bare.glb");
    touch(&asset);

    let mut db = AssetDatabase::new();
    let report = db.scan_directory(&dir.path).expect("scan");
    assert_eq!(report.registered, 0);
    assert!(db.is_empty());
}

#[test]
fn nested_directories_are_walked() {
    let dir = TempDir::new("nested");
    for sub in ["meshes", "textures", "audio/sfx"] {
        let asset = dir.path.join(sub).join("a.glb");
        touch(&asset);
        write_meta(&asset, &AssetMeta::new()).expect("write meta");
    }

    let mut db = AssetDatabase::new();
    let report = db.scan_directory(&dir.path).expect("scan");
    assert_eq!(report.registered, 3);
    assert_eq!(db.len(), 3);
}

#[test]
fn rescan_is_idempotent() {
    let dir = TempDir::new("rescan");
    let asset = dir.path.join("foo.glb");
    touch(&asset);
    write_meta(&asset, &AssetMeta::new()).expect("write meta");

    let mut db = AssetDatabase::new();
    db.scan_directory(&dir.path).expect("first scan");
    let report = db.scan_directory(&dir.path).expect("second scan");
    assert_eq!(report.registered, 0);
    assert_eq!(report.duplicates, 1);
    assert_eq!(db.len(), 1);
}

#[test]
fn entries_of_type_filters_correctly() {
    let mut db = AssetDatabase::new();
    let g_mesh = Guid::new_v4();
    let g_image = Guid::new_v4();
    let g_other = Guid::new_v4();
    db.register(
        g_mesh,
        AssetEntry {
            path: PathBuf::from("a.glb"),
            mtime: SystemTime::UNIX_EPOCH,
            type_name: Some("kooch_render::meshlet::MeshletMesh".to_owned()),
        },
    );
    db.register(
        g_image,
        AssetEntry {
            path: PathBuf::from("a.png"),
            mtime: SystemTime::UNIX_EPOCH,
            type_name: Some("kooch_render::texture::Image".to_owned()),
        },
    );
    db.register(
        g_other,
        AssetEntry {
            path: PathBuf::from("untyped.glb"),
            mtime: SystemTime::UNIX_EPOCH,
            type_name: None,
        },
    );

    let meshes: Vec<_> = db
        .entries_of_type("kooch_render::meshlet::MeshletMesh")
        .collect();
    assert_eq!(meshes.len(), 1);
    assert_eq!(meshes[0].0, g_mesh);

    let images: Vec<_> = db.entries_of_type("kooch_render::texture::Image").collect();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].0, g_image);

    let unknown: Vec<_> = db.entries_of_type("does::not::Exist").collect();
    assert!(unknown.is_empty());

    // Untyped entries do not match any type name — they show up
    // only when the picker explicitly looks for "" or via a
    // different surface that surfaces them.
    let none_filter: Vec<_> = db.entries_of_type("").collect();
    assert!(none_filter.is_empty());
}

#[test]
fn re_register_upgrades_type_name_from_none_to_some() {
    // Mirrors the real flow: scan_directory registers an asset
    // with `type_name = None` because the sidecar predates the
    // field. Later, `AssetServer::load::<MeshletMesh>` re-
    // registers the same `(path, guid)` pair with the freshly
    // back-filled type. The entry must end up typed.
    let mut db = AssetDatabase::new();
    let g = Guid::new_v4();
    let path = PathBuf::from("foo.glb");
    db.register(
        g,
        AssetEntry {
            path: path.clone(),
            mtime: SystemTime::UNIX_EPOCH,
            type_name: None,
        },
    );
    // Idempotent re-register with the same guid + path BUT a
    // populated type_name.
    let added = db.register(
        g,
        AssetEntry {
            path: path.clone(),
            mtime: SystemTime::UNIX_EPOCH,
            type_name: Some("kooch_render::meshlet::MeshletMesh".to_owned()),
        },
    );
    assert!(!added, "second register is not a brand-new entry");
    assert_eq!(
        db.entry(g).unwrap().type_name.as_deref(),
        Some("kooch_render::meshlet::MeshletMesh"),
        "type_name must upgrade from None to Some on re-register",
    );
}

#[test]
fn re_register_does_not_clobber_existing_type_name() {
    // Defensive: once a type is recorded, an idempotent re-
    // register that arrives without a type must NOT erase it.
    let mut db = AssetDatabase::new();
    let g = Guid::new_v4();
    let path = PathBuf::from("foo.glb");
    db.register(
        g,
        AssetEntry {
            path: path.clone(),
            mtime: SystemTime::UNIX_EPOCH,
            type_name: Some("KnownType".to_owned()),
        },
    );
    db.register(
        g,
        AssetEntry {
            path: path.clone(),
            mtime: SystemTime::UNIX_EPOCH,
            type_name: None,
        },
    );
    assert_eq!(
        db.entry(g).unwrap().type_name.as_deref(),
        Some("KnownType"),
        "untyped re-register must not erase a known type",
    );
}

#[test]
fn set_type_name_updates_existing_entry() {
    let mut db = AssetDatabase::new();
    let g = Guid::new_v4();
    db.register(
        g,
        AssetEntry {
            path: PathBuf::from("x.glb"),
            mtime: SystemTime::UNIX_EPOCH,
            type_name: None,
        },
    );
    assert!(db.set_type_name(g, "kooch_render::meshlet::MeshletMesh"));
    assert_eq!(
        db.entry(g).unwrap().type_name.as_deref(),
        Some("kooch_render::meshlet::MeshletMesh"),
    );
    // Idempotent re-write returns true and does not allocate.
    assert!(db.set_type_name(g, "kooch_render::meshlet::MeshletMesh"));
    // Unknown GUID returns false without registering anything new.
    assert!(!db.set_type_name(Guid::new_v4(), "Whatever"));
}

#[test]
fn register_replaces_when_path_guid_changes() {
    let mut db = AssetDatabase::new();
    let path = PathBuf::from("foo.glb");
    let g1 = Guid::new_v4();
    let g2 = Guid::new_v4();
    db.register(
        g1,
        AssetEntry {
            path: path.clone(),
            mtime: SystemTime::UNIX_EPOCH,
            type_name: None,
        },
    );
    db.register(
        g2,
        AssetEntry {
            path: path.clone(),
            mtime: SystemTime::UNIX_EPOCH,
            type_name: None,
        },
    );
    assert_eq!(db.len(), 1);
    assert_eq!(db.guid_for(&path), Some(g2));
    assert!(db.entry(g1).is_none(), "old GUID should be evicted");
    assert!(db.entry(g2).is_some());
}

#[test]
fn remove_path_drops_both_mappings() {
    let mut db = AssetDatabase::new();
    let path = PathBuf::from("tex/albedo.png");
    let guid = Guid::new_v4();
    db.register(
        guid,
        AssetEntry {
            path: path.clone(),
            mtime: SystemTime::UNIX_EPOCH,
            type_name: Some("kooch_render::texture::asset::Image".to_owned()),
        },
    );
    assert_eq!(db.len(), 1);

    assert_eq!(db.remove_path(&path), Some(guid));
    assert_eq!(db.len(), 0);
    assert!(db.guid_for(&path).is_none());
    assert!(db.entry(guid).is_none());
    // Removing again is a no-op.
    assert_eq!(db.remove_path(&path), None);
}
