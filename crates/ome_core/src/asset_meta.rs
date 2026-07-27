//! Sidecar `.meta` files — Unity-style asset metadata.
//!
//! Every asset under the project's `assets/` tree gets a sibling
//! `<file>.meta` file at first import. The `.meta` is the asset's
//! identity card: it carries the [`Guid`] that scenes and components
//! reference, plus (eventually) per-type import settings.
//!
//! Convention:
//! ```text
//! assets/meshes/suzanne.glb        # immutable source — engine never edits
//! assets/meshes/suzanne.glb.meta   # generated; contains GUID + import settings
//! ```
//!
//! Format is TOML — human-editable so artists can reassign GUIDs by
//! hand if they need to (Unity ships YAML; we ship the Rust-native
//! equivalent without dragging in a YAML parser).

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::guid::Guid;

/// Sidecar metadata for one asset file. Persisted as
/// `<asset>.meta` (TOML).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetMeta {
    /// Stable identifier for this asset. Survives renames and moves as
    /// long as the `.meta` file follows the source.
    pub guid: Guid,
    /// Concrete asset type the loader produced — `type_name::<T>()`
    /// from the loader registration. Optional because:
    /// 1. Sidecars created before this field existed must keep
    ///    parsing (back-compat with PR2 fixtures).
    /// 2. The directory scanner only reads `.meta` and does not
    ///    actually load the asset; type knowledge arrives the first
    ///    time `AssetServer::load::<T>` runs against the path.
    /// `AssetServer::load` back-fills the field whenever it finds an
    /// existing sidecar without one, so the steady state is always
    /// `Some(type_name)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_type: Option<String>,
}

impl AssetMeta {
    /// Builds metadata for a brand-new asset (fresh random GUID, no
    /// known type yet — `AssetServer::load::<T>` populates the type
    /// before serializing the sidecar to disk).
    pub fn new() -> Self {
        Self {
            guid: Guid::new_v4(),
            asset_type: None,
        }
    }

    /// Builds metadata with a fresh GUID and a known asset type.
    pub fn with_type(asset_type: impl Into<String>) -> Self {
        Self {
            guid: Guid::new_v4(),
            asset_type: Some(asset_type.into()),
        }
    }
}

impl Default for AssetMeta {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors produced while reading or writing a `.meta` sidecar.
#[derive(Debug)]
pub enum AssetMetaError {
    /// Filesystem I/O failed (file missing, permission, etc.).
    Io(std::io::Error),
    /// TOML deserialization failed — the sidecar is malformed.
    De(toml::de::Error),
    /// TOML serialization failed — should not happen for our schema,
    /// but `toml::ser::Error` is fallible at the type level.
    Ser(toml::ser::Error),
}

impl fmt::Display for AssetMetaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "asset meta I/O failed: {e}"),
            Self::De(e) => write!(f, "asset meta TOML parse failed: {e}"),
            Self::Ser(e) => write!(f, "asset meta TOML write failed: {e}"),
        }
    }
}

impl std::error::Error for AssetMetaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::De(e) => Some(e),
            Self::Ser(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for AssetMetaError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<toml::de::Error> for AssetMetaError {
    fn from(e: toml::de::Error) -> Self {
        Self::De(e)
    }
}

impl From<toml::ser::Error> for AssetMetaError {
    fn from(e: toml::ser::Error) -> Self {
        Self::Ser(e)
    }
}

/// Returns the sidecar path for `asset_path` — same directory, same
/// stem, with the extension chain extended by `.meta`.
///
/// `assets/meshes/suzanne.glb` → `assets/meshes/suzanne.glb.meta`.
pub fn meta_path_for(asset_path: &Path) -> PathBuf {
    let mut buf = asset_path.as_os_str().to_owned();
    buf.push(".meta");
    PathBuf::from(buf)
}

/// Reads the `.meta` sidecar living next to `asset_path` and parses it.
pub fn read_meta(asset_path: &Path) -> Result<AssetMeta, AssetMetaError> {
    let path = meta_path_for(asset_path);
    let text = fs::read_to_string(&path)?;
    let meta: AssetMeta = toml::from_str(&text)?;
    Ok(meta)
}

/// Writes `meta` to the `.meta` sidecar next to `asset_path`,
/// overwriting any existing file.
pub fn write_meta(asset_path: &Path, meta: &AssetMeta) -> Result<(), AssetMetaError> {
    let path = meta_path_for(asset_path);
    let text = toml::to_string_pretty(meta)?;
    fs::write(&path, text)?;
    Ok(())
}

/// Reads `<asset>.meta` if it exists; otherwise generates a fresh
/// `AssetMeta` and writes the sidecar before returning it. This is the
/// single entry point the asset server uses at first import.
pub fn read_or_create(asset_path: &Path) -> Result<AssetMeta, AssetMetaError> {
    let path = meta_path_for(asset_path);
    if path.exists() {
        return read_meta(asset_path);
    }
    let meta = AssetMeta::new();
    write_meta(asset_path, &meta)?;
    Ok(meta)
}

/// Type-aware variant of [`read_or_create`]. Same flow with three
/// extra guarantees:
///
/// - Sidecars created here always carry `asset_type = type_name`.
/// - Sidecars that already exist but lack `asset_type` get the field
///   back-filled and rewritten to disk.
/// - Sidecars whose existing `asset_type` differs from `type_name`
///   are left as-is and returned untouched — the caller decides
///   whether to treat the mismatch as an error (typically yes for
///   `AssetServer::load::<T>` because `T` ought to match the file's
///   recorded type).
pub fn read_or_create_typed(
    asset_path: &Path,
    type_name: &str,
) -> Result<AssetMeta, AssetMetaError> {
    let path = meta_path_for(asset_path);
    if !path.exists() {
        let meta = AssetMeta::with_type(type_name);
        write_meta(asset_path, &meta)?;
        return Ok(meta);
    }
    let mut meta = read_meta(asset_path)?;
    if meta.asset_type.is_none() {
        meta.asset_type = Some(type_name.to_owned());
        write_meta(asset_path, &meta)?;
    }
    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Minimal RAII tempdir — we don't pull `tempfile` into ome_core
    /// for one test surface. Best-effort cleanup on drop.
    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new(name: &str) -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!("ome_asset_meta_{name}_{}", std::process::id(),));
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

        let original = AssetMeta::with_type("ome_render::meshlet::MeshletMesh");
        write_meta(&asset, &original).expect("write");
        let parsed = read_meta(&asset).expect("read");

        assert_eq!(parsed, original);
        assert_eq!(
            parsed.asset_type.as_deref(),
            Some("ome_render::meshlet::MeshletMesh"),
        );
    }

    #[test]
    fn read_or_create_typed_writes_type_on_first_create() {
        let dir = TempDir::new("typed_create");
        let asset = dir.path.join("foo.glb");
        touch(&asset);

        let meta =
            read_or_create_typed(&asset, "ome_render::meshlet::MeshletMesh").expect("create typed");
        assert_eq!(
            meta.asset_type.as_deref(),
            Some("ome_render::meshlet::MeshletMesh"),
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

        let meta = read_or_create_typed(&asset, "ome_render::texture::Image")
            .expect("read existing then backfill");
        assert_eq!(
            meta.asset_type.as_deref(),
            Some("ome_render::texture::Image"),
        );

        // Re-read to confirm the change persisted to disk.
        let on_disk = read_meta(&asset).expect("re-read");
        assert_eq!(
            on_disk.asset_type.as_deref(),
            Some("ome_render::texture::Image"),
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
        let original = AssetMeta::with_type("ome_render::meshlet::MeshletMesh");
        write_meta(&asset, &original).expect("seed typed meta");

        let parsed = read_or_create_typed(&asset, "ome_render::texture::Image")
            .expect("read existing typed");
        assert_eq!(
            parsed.asset_type.as_deref(),
            Some("ome_render::meshlet::MeshletMesh"),
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
}
