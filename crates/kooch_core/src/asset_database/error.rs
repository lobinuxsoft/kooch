use std::path::PathBuf;

use crate::asset_meta::AssetMetaError;

/// Errors produced while scanning or registering with the database.
#[derive(Debug)]
pub enum AssetDatabaseError {
    /// Filesystem walk failed.
    Io(std::io::Error),
    /// A sidecar parsed but referenced an asset whose source file is
    /// missing. The database refuses to register an orphan entry.
    OrphanSidecar(PathBuf),
    /// Reading or parsing a `.meta` file failed.
    Meta(AssetMetaError),
}

impl std::fmt::Display for AssetDatabaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "asset database I/O failed: {e}"),
            Self::OrphanSidecar(p) => {
                write!(f, "sidecar at {p:?} has no matching source asset")
            }
            Self::Meta(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AssetDatabaseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::OrphanSidecar(_) => None,
            Self::Meta(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for AssetDatabaseError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<AssetMetaError> for AssetDatabaseError {
    fn from(e: AssetMetaError) -> Self {
        Self::Meta(e)
    }
}
