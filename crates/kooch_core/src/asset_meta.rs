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
    /// Per-type import settings, verbatim.
    ///
    /// Kept as an opaque table because what belongs in it is the
    /// loader's business and not this crate's: a texture says whether
    /// it wants a mip chain, a mesh would say what its units are.
    /// [`LoadContext::import`](crate::asset_loader::LoadContext::import)
    /// hands it to whoever owns the type.
    ///
    /// 🔴 Absent means "the engine's default", NOT "everything off".
    /// A sidecar written before this field existed keeps parsing and
    /// keeps behaving the way it did — which is the same rule
    /// `asset_type` follows above, for the same reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import: Option<toml::Table>,
}

impl AssetMeta {
    /// Builds metadata for a brand-new asset (fresh random GUID, no
    /// known type yet — `AssetServer::load::<T>` populates the type
    /// before serializing the sidecar to disk).
    pub fn new() -> Self {
        Self {
            guid: Guid::new_v4(),
            asset_type: None,
            import: None,
        }
    }

    /// Builds metadata with a fresh GUID and a known asset type.
    pub fn with_type(asset_type: impl Into<String>) -> Self {
        Self {
            guid: Guid::new_v4(),
            asset_type: Some(asset_type.into()),
            import: None,
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
mod tests;
