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
}

impl AssetMeta {
    /// Builds metadata for a brand-new asset (fresh random GUID).
    pub fn new() -> Self {
        Self {
            guid: Guid::new_v4(),
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
            path.push(format!(
                "ome_asset_meta_{name}_{}",
                std::process::id(),
            ));
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
}
