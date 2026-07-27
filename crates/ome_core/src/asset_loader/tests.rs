use super::error::{AssetError, AssetResult};
use super::server::AssetServer;
use super::trait_def::{AssetLoader, LoadContext};
use crate::asset_database::AssetDatabase;
use crate::asset_meta;
use crate::assets::Assets;
use crate::guid::Guid;
use crate::resource::Resources;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq)]
struct PlainText(String);

/// Loader that copies bytes into a UTF-8 string.
struct TextLoader;
impl AssetLoader<PlainText> for TextLoader {
    fn extensions(&self) -> &[&'static str] {
        &["txt", "log"]
    }

    fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<PlainText> {
        let s = std::str::from_utf8(bytes)
            .map_err(|e| AssetError::Loader(Box::new(e)))?
            .to_string();
        Ok(PlainText(s))
    }
}

#[derive(Debug, PartialEq)]
struct Other(u32);

fn temp_file_with(content: &str, ext: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let filename = format!(
        "ome_assetloader_test_{}.{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        ext,
    );
    path.push(filename);
    let mut f = std::fs::File::create(&path).expect("temp file");
    f.write_all(content.as_bytes()).expect("write temp");
    path
}

#[test]
fn register_then_has_loader() {
    let mut server = AssetServer::new();
    assert!(!server.has_loader::<PlainText>());
    server.register_loader::<PlainText, _>(TextLoader);
    assert!(server.has_loader::<PlainText>());
}

#[test]
fn extensions_reflect_registered_loader() {
    let mut server = AssetServer::new();
    server.register_loader::<PlainText, _>(TextLoader);
    let exts = server.extensions_for::<PlainText>();
    assert_eq!(exts, &["txt", "log"]);
}

#[test]
fn extensions_empty_when_no_loader() {
    let server = AssetServer::new();
    assert!(server.extensions_for::<PlainText>().is_empty());
}

#[test]
fn load_without_registered_loader_errs() {
    let mut server = AssetServer::new();
    let mut resources = Resources::new();
    resources.insert(Assets::<PlainText>::new());
    let err = server
        .load::<PlainText>("anything.txt", &mut resources)
        .unwrap_err();
    match err {
        AssetError::NoLoaderForType(name) => {
            assert!(name.contains("PlainText"));
        }
        other => panic!("expected NoLoaderForType, got {other:?}"),
    }
}

#[test]
fn load_with_unsupported_extension_errs() {
    let mut server = AssetServer::new();
    server.register_loader::<PlainText, _>(TextLoader);
    let mut resources = Resources::new();
    resources.insert(Assets::<PlainText>::new());
    let path = temp_file_with("data", "bin");
    let err = server.load::<PlainText>(&path, &mut resources).unwrap_err();
    match err {
        AssetError::UnsupportedExtension { registered, .. } => {
            assert_eq!(registered, vec!["txt", "log"]);
        }
        other => panic!("expected UnsupportedExtension, got {other:?}"),
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn load_succeeds_and_caches_handle() {
    let mut server = AssetServer::new();
    server.register_loader::<PlainText, _>(TextLoader);
    let mut resources = Resources::new();
    resources.insert(Assets::<PlainText>::new());
    let path = temp_file_with("hello world", "txt");

    let h1 = server.load::<PlainText>(&path, &mut resources).unwrap();
    let h2 = server.load::<PlainText>(&path, &mut resources).unwrap();
    assert_eq!(h1, h2, "second load should hit cache, not re-insert");

    let assets = resources.get::<Assets<PlainText>>().unwrap();
    assert_eq!(assets.len(), 1);
    assert_eq!(assets.get(h1).map(|t| t.0.as_str()), Some("hello world"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn get_cached_returns_none_before_load() {
    let server = AssetServer::new();
    let path = std::env::temp_dir().join("never_loaded.txt");
    assert!(server.get_cached::<PlainText>(&path).is_none());
}

#[test]
fn get_cached_returns_handle_after_load() {
    let mut server = AssetServer::new();
    server.register_loader::<PlainText, _>(TextLoader);
    let mut resources = Resources::new();
    resources.insert(Assets::<PlainText>::new());
    let path = temp_file_with("cached", "log");

    let handle = server.load::<PlainText>(&path, &mut resources).unwrap();
    let cached = server.get_cached::<PlainText>(&path).unwrap();
    assert_eq!(handle, cached);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn missing_assets_storage_errs() {
    let mut server = AssetServer::new();
    server.register_loader::<PlainText, _>(TextLoader);
    let mut resources = Resources::new(); // no Assets<PlainText> inserted
    let path = temp_file_with("data", "txt");

    let err = server.load::<PlainText>(&path, &mut resources).unwrap_err();
    match err {
        AssetError::MissingAssetStorage(name) => {
            assert!(name.contains("PlainText"));
        }
        other => panic!("expected MissingAssetStorage, got {other:?}"),
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn loader_per_type_is_independent() {
    struct OtherLoader;
    impl AssetLoader<Other> for OtherLoader {
        fn extensions(&self) -> &[&'static str] {
            &["bin"]
        }
        fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<Other> {
            if bytes.len() < 4 {
                return Err(AssetError::Loader("need 4 bytes".into()));
            }
            let value = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            Ok(Other(value))
        }
    }

    let mut server = AssetServer::new();
    server.register_loader::<PlainText, _>(TextLoader);
    server.register_loader::<Other, _>(OtherLoader);

    assert!(server.has_loader::<PlainText>());
    assert!(server.has_loader::<Other>());
    assert_eq!(server.extensions_for::<PlainText>(), &["txt", "log"]);
    assert_eq!(server.extensions_for::<Other>(), &["bin"]);
}

#[test]
fn relative_path_resolves_against_root() {
    let server = AssetServer::new().with_asset_root("/tmp/some/root");
    let resolved = server.resolve_path(Path::new("models/cube.glb"));
    assert_eq!(resolved, PathBuf::from("/tmp/some/root/models/cube.glb"));
}

#[test]
fn absolute_path_bypasses_root() {
    let server = AssetServer::new().with_asset_root("/tmp/some/root");
    let resolved = server.resolve_path(Path::new("/etc/passwd"));
    assert_eq!(resolved, PathBuf::from("/etc/passwd"));
}

#[test]
fn no_root_keeps_path_raw() {
    let server = AssetServer::new();
    let resolved = server.resolve_path(Path::new("models/cube.glb"));
    assert_eq!(resolved, PathBuf::from("models/cube.glb"));
}

#[test]
fn clear_cache_drops_path_lookup_only() {
    let mut server = AssetServer::new();
    server.register_loader::<PlainText, _>(TextLoader);
    let mut resources = Resources::new();
    resources.insert(Assets::<PlainText>::new());
    let path = temp_file_with("data", "txt");

    let _h = server.load::<PlainText>(&path, &mut resources).unwrap();
    assert!(server.get_cached::<PlainText>(&path).is_some());
    server.clear_cache();
    assert!(server.get_cached::<PlainText>(&path).is_none());

    // Storage untouched
    let assets = resources.get::<Assets<PlainText>>().unwrap();
    assert_eq!(assets.len(), 1);
    let _ = std::fs::remove_file(&path);
}

/// Load on a path with no `.meta` adjacent should generate one
/// transparently; the sidecar must exist after the call.
#[test]
fn load_generates_meta_on_first_call() {
    let mut server = AssetServer::new();
    server.register_loader::<PlainText, _>(TextLoader);
    let mut resources = Resources::new();
    resources.insert(Assets::<PlainText>::new());
    let path = temp_file_with("hello", "txt");
    let meta_path = asset_meta::meta_path_for(&path);
    assert!(
        !meta_path.exists(),
        "test fixture must start without sidecar"
    );

    server.load::<PlainText>(&path, &mut resources).unwrap();
    assert!(meta_path.exists(), "load should have generated the sidecar");

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&meta_path);
}

/// When `AssetDatabase` is in resources, a successful load must
/// register the asset under the GUID found in the `.meta`.
#[test]
fn load_registers_in_database_when_present() {
    let mut server = AssetServer::new();
    server.register_loader::<PlainText, _>(TextLoader);
    let mut resources = Resources::new();
    resources.insert(Assets::<PlainText>::new());
    resources.insert(AssetDatabase::new());
    let path = temp_file_with("hello", "txt");
    let meta_path = asset_meta::meta_path_for(&path);

    server.load::<PlainText>(&path, &mut resources).unwrap();

    let db = resources.get::<AssetDatabase>().unwrap();
    let guid = db.guid_for(&path).expect("path should be registered");
    let entry = db.entry(guid).expect("guid should resolve");
    assert_eq!(entry.path, path);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&meta_path);
}

/// Load must not require an `AssetDatabase` resource — the
/// integration is opportunistic. Without one, the sidecar still
/// gets generated but no registration happens.
#[test]
fn load_works_without_database() {
    let mut server = AssetServer::new();
    server.register_loader::<PlainText, _>(TextLoader);
    let mut resources = Resources::new();
    resources.insert(Assets::<PlainText>::new());
    let path = temp_file_with("hello", "txt");
    let meta_path = asset_meta::meta_path_for(&path);

    let _ = server
        .load::<PlainText>(&path, &mut resources)
        .expect("load should succeed without AssetDatabase");
    assert!(meta_path.exists());

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&meta_path);
}

/// `load_by_guid` requires an `AssetDatabase` resource — without
/// one, it cannot resolve GUID → path.
#[test]
fn load_by_guid_without_database_errs() {
    let mut server = AssetServer::new();
    server.register_loader::<PlainText, _>(TextLoader);
    let mut resources = Resources::new();
    resources.insert(Assets::<PlainText>::new());
    let err = server
        .load_by_guid::<PlainText>(Guid::new_v4(), &mut resources)
        .unwrap_err();
    assert!(matches!(
        err,
        AssetError::MissingAssetStorage(name) if name.contains("AssetDatabase")
    ));
}

/// Unknown GUIDs surface as `UnknownGuid`.
#[test]
fn load_by_guid_unknown_guid_errs() {
    let mut server = AssetServer::new();
    server.register_loader::<PlainText, _>(TextLoader);
    let mut resources = Resources::new();
    resources.insert(Assets::<PlainText>::new());
    resources.insert(AssetDatabase::new());
    let stranger = Guid::new_v4();
    let err = server
        .load_by_guid::<PlainText>(stranger, &mut resources)
        .unwrap_err();
    assert!(matches!(err, AssetError::UnknownGuid(g) if g == stranger));
}

/// `load` followed by `load_by_guid` for the same asset must
/// return the same handle — the path-keyed cache short-circuits
/// the second call.
#[test]
fn load_then_load_by_guid_returns_cached_handle() {
    let mut server = AssetServer::new();
    server.register_loader::<PlainText, _>(TextLoader);
    let mut resources = Resources::new();
    resources.insert(Assets::<PlainText>::new());
    resources.insert(AssetDatabase::new());
    let path = temp_file_with("hello", "txt");
    let meta_path = asset_meta::meta_path_for(&path);

    let h_via_path = server.load::<PlainText>(&path, &mut resources).unwrap();
    let guid = resources
        .get::<AssetDatabase>()
        .unwrap()
        .guid_for(&path)
        .unwrap();
    let h_via_guid = server
        .load_by_guid::<PlainText>(guid, &mut resources)
        .unwrap();

    assert_eq!(h_via_path, h_via_guid);
    let assets = resources.get::<Assets<PlainText>>().unwrap();
    assert_eq!(assets.len(), 1, "no duplicate insert");

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&meta_path);
}

/// Reloading the same asset must reuse the existing sidecar — the
/// GUID must remain stable.
#[test]
fn second_load_keeps_same_guid() {
    let mut server = AssetServer::new();
    server.register_loader::<PlainText, _>(TextLoader);
    let mut resources = Resources::new();
    resources.insert(Assets::<PlainText>::new());
    resources.insert(AssetDatabase::new());
    let path = temp_file_with("hello", "txt");
    let meta_path = asset_meta::meta_path_for(&path);

    server.load::<PlainText>(&path, &mut resources).unwrap();
    let first_guid = resources
        .get::<AssetDatabase>()
        .unwrap()
        .guid_for(&path)
        .unwrap();

    // Force a re-load (clear path cache; sidecar persists on disk).
    server.clear_cache();
    server.load::<PlainText>(&path, &mut resources).unwrap();
    let second_guid = resources
        .get::<AssetDatabase>()
        .unwrap()
        .guid_for(&path)
        .unwrap();

    assert_eq!(first_guid, second_guid, "GUID must be stable across loads");

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&meta_path);
}
