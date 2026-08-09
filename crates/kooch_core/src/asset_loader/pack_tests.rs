//! #758 — a shipped game reads its assets out of a `.kpack`.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use kooch_pack::{PackKey, PackWriter};

use super::{AssetError, AssetLoader, AssetResult, AssetServer, LoadContext};
use crate::assets::Assets;
use crate::resource::Resources;

/// An asset that is its own bytes, so a test asserts on what the server
/// read rather than on what some parser made of it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Blob(Vec<u8>);

#[derive(Default)]
struct BlobLoader;

impl AssetLoader<Blob> for BlobLoader {
    fn extensions(&self) -> &[&'static str] {
        &["blob"]
    }

    fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<Blob> {
        Ok(Blob(bytes.to_vec()))
    }
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kooch_packread_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes a pack over `root` holding `files`.
fn write_pack(dir: &Path, key: &PackKey, files: &[(&str, &[u8])]) -> PathBuf {
    let mut writer = PackWriter::new(Cursor::new(Vec::new()), key).unwrap();
    for (name, bytes) in files {
        writer.add(name, bytes).unwrap();
    }
    let path = dir.join("assets.kpack");
    std::fs::write(&path, writer.finish().unwrap().into_inner()).unwrap();
    path
}

fn server() -> (AssetServer, Resources) {
    let mut server = AssetServer::new();
    server.register_loader::<Blob, _>(BlobLoader);
    let mut resources = Resources::new();
    resources.insert(Assets::<Blob>::new());
    (server, resources)
}

fn load(server: &mut AssetServer, resources: &mut Resources, path: &Path) -> AssetResult<Vec<u8>> {
    let handle = server.load::<Blob>(path, resources)?;
    Ok(resources
        .get::<Assets<Blob>>()
        .unwrap()
        .get(handle)
        .unwrap()
        .0
        .clone())
}

#[test]
fn an_asset_comes_out_of_a_mounted_pack() {
    let dir = tmp("mounted");
    let key = PackKey::generate();
    let pack = write_pack(&dir, &key, &[("thing.blob", b"from the pack")]);
    let (mut server, mut resources) = server();

    let entries = server.mount_pack(dir.clone(), &pack, &key).unwrap();

    assert_eq!(entries, 1);
    assert!(server.has_packs());
    // Nothing of that name exists on disk.
    assert!(!dir.join("thing.blob").exists());
    assert_eq!(
        load(&mut server, &mut resources, &dir.join("thing.blob")).unwrap(),
        b"from the pack",
    );
}

/// 🔴 The pack is what a release shipped with, so it wins over whatever
/// happens to be lying beside the executable.
#[test]
fn the_pack_wins_over_a_loose_file() {
    let dir = tmp("shadow");
    let key = PackKey::generate();
    std::fs::write(dir.join("thing.blob"), b"from the disk").unwrap();
    let pack = write_pack(&dir, &key, &[("thing.blob", b"from the pack")]);
    let (mut server, mut resources) = server();
    server.mount_pack(dir.clone(), &pack, &key).unwrap();

    assert_eq!(
        load(&mut server, &mut resources, &dir.join("thing.blob")).unwrap(),
        b"from the pack",
    );
}

/// Development is unaffected: nothing mounted, the disk answers exactly
/// as before.
#[test]
fn without_a_pack_the_disk_answers() {
    let dir = tmp("disk");
    std::fs::write(dir.join("thing.blob"), b"from the disk").unwrap();
    let (mut server, mut resources) = server();

    assert!(!server.has_packs());
    assert_eq!(
        load(&mut server, &mut resources, &dir.join("thing.blob")).unwrap(),
        b"from the disk",
    );
}

/// A path a pack does not hold falls through, so a mounted pack does not
/// hide the rest of the filesystem.
#[test]
fn a_path_the_pack_lacks_falls_through() {
    let dir = tmp("fallthrough");
    let key = PackKey::generate();
    let pack = write_pack(&dir, &key, &[("packed.blob", b"packed")]);
    std::fs::write(dir.join("loose.blob"), b"loose").unwrap();
    let (mut server, mut resources) = server();
    server.mount_pack(dir.clone(), &pack, &key).unwrap();

    assert_eq!(
        load(&mut server, &mut resources, &dir.join("loose.blob")).unwrap(),
        b"loose",
    );
}

/// Outside every mounted root is not in a pack, and that is an ordinary
/// answer rather than an error.
#[test]
fn a_path_outside_the_root_is_not_packed() {
    let dir = tmp("outside");
    let elsewhere = tmp("outside_other");
    let key = PackKey::generate();
    let pack = write_pack(&dir, &key, &[("thing.blob", b"packed")]);
    std::fs::write(elsewhere.join("thing.blob"), b"elsewhere").unwrap();
    let (mut server, mut resources) = server();
    server.mount_pack(dir.clone(), &pack, &key).unwrap();

    assert_eq!(
        load(&mut server, &mut resources, &elsewhere.join("thing.blob")).unwrap(),
        b"elsewhere",
    );
}

/// 🔴 In the pack and unreadable must not fall through to the disk. A
/// shipped game has nothing there, and the error a player eventually sees
/// should say the pack is damaged rather than name a missing file.
#[test]
fn a_damaged_entry_does_not_fall_through() {
    let dir = tmp("damaged");
    let key = PackKey::generate();
    let pack = write_pack(
        &dir,
        &key,
        &[("thing.blob", b"a payload long enough to hurt")],
    );
    // A loose file that must NOT be used as a silent substitute.
    std::fs::write(dir.join("thing.blob"), b"from the disk").unwrap();

    let mut bytes = std::fs::read(&pack).unwrap();
    let at = bytes.len() / 2;
    bytes[at] ^= 0xff;
    std::fs::write(&pack, &bytes).unwrap();

    let (mut server, mut resources) = server();
    // Mounting may already fail if the damage landed in the index; both
    // outcomes are correct, and neither may be "read the disk instead".
    if server.mount_pack(dir.clone(), &pack, &key).is_ok() {
        let read = load(&mut server, &mut resources, &dir.join("thing.blob"));
        match read {
            Err(AssetError::Loader(_)) => {}
            Ok(bytes) => panic!(
                "a damaged pack entry was served as {:?}",
                String::from_utf8_lossy(&bytes),
            ),
            Err(e) => panic!("unexpected error: {e}"),
        }
    }
}

/// The wrong key cannot mount, rather than mounting empty and letting
/// every asset quietly miss.
#[test]
fn the_wrong_key_refuses_to_mount() {
    let dir = tmp("wrongkey");
    let pack = write_pack(&dir, &PackKey::generate(), &[("thing.blob", b"x")]);
    let (mut server, _) = server();

    assert!(server.mount_pack(dir, &pack, &PackKey::generate()).is_err());
    assert!(!server.has_packs());
}
