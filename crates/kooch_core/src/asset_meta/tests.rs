use super::*;
use std::io::Write;

/// Minimal RAII tempdir — we don't pull `tempfile` into kooch_core
/// for one test surface. Best-effort cleanup on drop.
struct TempDir {
    path: PathBuf,
}
impl TempDir {
    fn new(name: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!("kooch_asset_meta_{name}_{}", std::process::id(),));
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
    let mut f = fs::File::create(path).expect("create source asset");
    f.write_all(b"placeholder").expect("write");
}

#[test]
fn meta_path_appends_dot_meta_to_full_filename() {
    let p = Path::new("assets/meshes/suzanne.glb");
    assert_eq!(
        meta_path_for(p),
        PathBuf::from("assets/meshes/suzanne.glb.meta"),
    );
}

#[test]
fn round_trip_persists_guid() {
    let dir = TempDir::new("round_trip");
    let asset = dir.path.join("foo.glb");
    touch(&asset);

    let original = AssetMeta::new();
    write_meta(&asset, &original).expect("write");
    let parsed = read_meta(&asset).expect("read");

    assert_eq!(parsed, original);
}

#[test]
fn read_or_create_generates_when_missing() {
    let dir = TempDir::new("create_missing");
    let asset = dir.path.join("bar.glb");
    touch(&asset);

    assert!(!meta_path_for(&asset).exists());
    let meta = read_or_create(&asset).expect("create on first read");
    assert!(meta_path_for(&asset).exists(), "sidecar should be written");

    // Second call returns the SAME GUID — does not regenerate.
    let again = read_or_create(&asset).expect("read existing");
    assert_eq!(meta.guid, again.guid);
}

#[test]
fn read_meta_io_error_on_missing_sidecar() {
    let dir = TempDir::new("missing_sidecar");
    let asset = dir.path.join("nope.glb");
    touch(&asset);

    let err = read_meta(&asset).expect_err("no sidecar to read");
    assert!(matches!(err, AssetMetaError::Io(_)));
}

#[test]
fn read_meta_de_error_on_garbage() {
    let dir = TempDir::new("garbage");
    let asset = dir.path.join("baz.glb");
    touch(&asset);
    fs::write(meta_path_for(&asset), "this is not toml = = =").expect("seed garbage");

    let err = read_meta(&asset).expect_err("garbage should fail to parse");
    assert!(matches!(err, AssetMetaError::De(_)));
}

#[test]
fn legacy_sidecar_without_asset_type_still_parses() {
    // Pre-PR4 sidecars only carry `guid`. New code must continue
    // to read them — the missing field falls through to None.
    let dir = TempDir::new("legacy");
    let asset = dir.path.join("legacy.glb");
    touch(&asset);
    let raw = "guid = \"550e8400e29b41d4a716446655440000\"\n";
    fs::write(meta_path_for(&asset), raw).expect("write legacy meta");

    let meta = read_meta(&asset).expect("legacy meta must parse");
    assert!(meta.asset_type.is_none());
}

#[test]
fn with_type_round_trips_asset_type_field() {
    let dir = TempDir::new("typed");
    let asset = dir.path.join("typed.glb");
    touch(&asset);

    let original = AssetMeta::with_type("kooch_render::meshlet::MeshletMesh");
    write_meta(&asset, &original).expect("write");
    let parsed = read_meta(&asset).expect("read");

    assert_eq!(parsed, original);
    assert_eq!(
        parsed.asset_type.as_deref(),
        Some("kooch_render::meshlet::MeshletMesh"),
    );
}

#[test]
fn read_or_create_typed_writes_type_on_first_create() {
    let dir = TempDir::new("typed_create");
    let asset = dir.path.join("foo.glb");
    touch(&asset);

    let meta =
        read_or_create_typed(&asset, "kooch_render::meshlet::MeshletMesh").expect("create typed");
    assert_eq!(
        meta.asset_type.as_deref(),
        Some("kooch_render::meshlet::MeshletMesh"),
    );

    // Sidecar on disk also carries the type.
    let raw = fs::read_to_string(meta_path_for(&asset)).unwrap();
    assert!(raw.contains("asset_type"));
}

#[test]
fn read_or_create_typed_backfills_existing_untyped_sidecar() {
    let dir = TempDir::new("typed_backfill");
    let asset = dir.path.join("legacy.glb");
    touch(&asset);
    let raw = "guid = \"550e8400e29b41d4a716446655440000\"\n";
    fs::write(meta_path_for(&asset), raw).expect("seed legacy meta");

    let meta = read_or_create_typed(&asset, "kooch_render::texture::Image")
        .expect("read existing then backfill");
    assert_eq!(
        meta.asset_type.as_deref(),
        Some("kooch_render::texture::Image"),
    );

    // Re-read to confirm the change persisted to disk.
    let on_disk = read_meta(&asset).expect("re-read");
    assert_eq!(
        on_disk.asset_type.as_deref(),
        Some("kooch_render::texture::Image"),
    );
}

#[test]
fn read_or_create_typed_preserves_existing_type_mismatch() {
    // The function intentionally does NOT overwrite an existing
    // type — it backfills only when the field is None. The
    // caller can detect a mismatch by comparing the returned
    // value to the type they passed.
    let dir = TempDir::new("typed_mismatch");
    let asset = dir.path.join("typed.glb");
    touch(&asset);
    let original = AssetMeta::with_type("kooch_render::meshlet::MeshletMesh");
    write_meta(&asset, &original).expect("seed typed meta");

    let parsed =
        read_or_create_typed(&asset, "kooch_render::texture::Image").expect("read existing typed");
    assert_eq!(
        parsed.asset_type.as_deref(),
        Some("kooch_render::meshlet::MeshletMesh"),
        "must NOT overwrite an existing recorded type",
    );
}

#[test]
fn untyped_meta_omits_asset_type_in_toml() {
    // Sidecars generated before the loader knows the type must
    // not pollute the file with `asset_type = ""` — they should
    // only carry `guid`. Confirmed by checking the serialized form.
    let meta = AssetMeta::new();
    let text = toml::to_string(&meta).expect("serialize");
    assert!(text.contains("guid"));
    assert!(
        !text.contains("asset_type"),
        "untyped meta must skip the asset_type field, got:\n{text}",
    );
}
