//! #758 — a packaged game answers "which file is GUID `abc`".

use std::io::Cursor;
use std::path::{Path, PathBuf};

use kooch_pack::{PackKey, PackWriter};

use super::*;
use crate::asset_meta::AssetMeta;
use crate::guid::Guid;

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kooch_packscan_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A pack holding `files`, each `(name, bytes, Option<meta>)`.
fn pack_with(
    dir: &Path,
    key: &PackKey,
    files: &[(&str, &[u8], Option<AssetMeta>)],
) -> (PathBuf, PathBuf) {
    let mut writer = PackWriter::new(Cursor::new(Vec::new()), key).unwrap();
    for (name, bytes, meta) in files {
        writer.add(name, bytes).unwrap();
        if let Some(meta) = meta {
            let text = toml::to_string_pretty(meta).unwrap();
            writer
                .add(&format!("{name}.meta"), text.as_bytes())
                .unwrap();
        }
    }
    let path = dir.join("assets.kpack");
    std::fs::write(&path, writer.finish().unwrap().into_inner()).unwrap();
    (dir.join("assets"), path)
}

fn mounted(root: PathBuf, pack: &Path, key: &PackKey) -> AssetServer {
    let mut server = AssetServer::new();
    server.mount_pack(root, pack, key).unwrap();
    server
}

/// 🔴 The question mounting does not answer. Without this the game reads
/// its pack perfectly and resolves not one GUID, so a scene spawns
/// entities with no meshes and no materials on them.
#[test]
fn packed_assets_are_registered_by_guid() {
    let dir = tmp("guids");
    let key = PackKey::generate();
    let guid = Guid::new_v4();
    let (root, pack) = pack_with(
        &dir,
        &key,
        &[(
            "props/rock.glb",
            b"mesh",
            Some(AssetMeta {
                guid,
                asset_type: Some("Mesh".to_owned()),
                import: None,
            }),
        )],
    );
    let mut server = mounted(root.clone(), &pack, &key);
    let mut database = AssetDatabase::new();

    let scan = scan_packs(&mut server, &mut database);

    assert_eq!(scan.registered, 1);
    assert_eq!(scan.orphans, 0);
    let entry = database.entry(guid).expect("the GUID resolves");
    assert_eq!(entry.path, root.join("props/rock.glb"));
    assert_eq!(entry.type_name.as_deref(), Some("Mesh"));
}

/// And the path it registers is one the server can then read, which is
/// the whole point of the round trip.
#[test]
fn the_registered_path_is_readable() {
    let dir = tmp("readable");
    let key = PackKey::generate();
    let guid = Guid::new_v4();
    let (root, pack) = pack_with(
        &dir,
        &key,
        &[(
            "props/rock.glb",
            b"mesh bytes",
            Some(AssetMeta {
                guid,
                asset_type: None,
                import: None,
            }),
        )],
    );
    let mut server = mounted(root, &pack, &key);
    let mut database = AssetDatabase::new();
    scan_packs(&mut server, &mut database);

    let path = database.entry(guid).unwrap().path.clone();
    assert_eq!(server.read_packed(&path).unwrap(), b"mesh bytes");
}

/// A sidecar is not an asset. Registering `rock.glb.meta` as a thing in
/// its own right would double the database and put files in the browser
/// that are not assets.
#[test]
fn sidecars_are_not_registered_as_assets() {
    let dir = tmp("nosidecar");
    let key = PackKey::generate();
    let (root, pack) = pack_with(
        &dir,
        &key,
        &[(
            "a.glb",
            b"x",
            Some(AssetMeta {
                guid: Guid::new_v4(),
                asset_type: None,
                import: None,
            }),
        )],
    );
    let mut server = mounted(root, &pack, &key);
    let mut database = AssetDatabase::new();

    let scan = scan_packs(&mut server, &mut database);

    assert_eq!(scan.registered, 1);
    assert_eq!(database.len(), 1);
}

/// A file nothing references by GUID is not an error — but it is
/// counted, because all of them being orphans means the sidecars did not
/// travel.
#[test]
fn a_file_without_a_sidecar_is_an_orphan() {
    let dir = tmp("orphan");
    let key = PackKey::generate();
    let (root, pack) = pack_with(&dir, &key, &[("readme.txt", b"hello", None)]);
    let mut server = mounted(root, &pack, &key);
    let mut database = AssetDatabase::new();

    let scan = scan_packs(&mut server, &mut database);

    assert_eq!(scan.registered, 0);
    assert_eq!(scan.orphans, 1);
    assert!(database.is_empty());
}

/// A sidecar that will not parse must not take the scan down with it.
#[test]
fn a_malformed_sidecar_is_an_orphan() {
    let dir = tmp("malformed");
    let key = PackKey::generate();
    let mut writer = PackWriter::new(Cursor::new(Vec::new()), &key).unwrap();
    writer.add("a.glb", b"x").unwrap();
    writer.add("a.glb.meta", b"not toml {{{").unwrap();
    writer
        .add(
            "b.glb.meta",
            toml::to_string_pretty(&AssetMeta {
                guid: Guid::new_v4(),
                asset_type: None,
                import: None,
            })
            .unwrap()
            .as_bytes(),
        )
        .unwrap();
    writer.add("b.glb", b"y").unwrap();
    let path = dir.join("assets.kpack");
    std::fs::write(&path, writer.finish().unwrap().into_inner()).unwrap();

    let mut server = mounted(dir.join("assets"), &path, &key);
    let mut database = AssetDatabase::new();

    let scan = scan_packs(&mut server, &mut database);

    assert_eq!(scan.registered, 1, "the good sidecar was skipped too");
    assert_eq!(scan.orphans, 1);
}

/// Nothing mounted is nothing to scan, and no complaint: that is a
/// project being edited, where the disk scan does this job.
#[test]
fn no_packs_is_an_empty_scan() {
    let mut server = AssetServer::new();
    let mut database = AssetDatabase::new();

    assert_eq!(scan_packs(&mut server, &mut database), PackScan::default(),);
}
