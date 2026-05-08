use crate::guid::Guid;
use std::fmt;
use std::path::PathBuf;

/// Result alias for asset operations.
pub type AssetResult<T> = Result<T, AssetError>;

/// Errors produced during load. Loader implementations wrap their domain
/// errors in [`AssetError::Loader`].
#[derive(Debug)]
pub enum AssetError {
    /// File could not be opened or read.
    Io(std::io::Error),
    /// No loader registered for the requested asset type.
    NoLoaderForType(&'static str),
    /// File extension is not supported by the registered loader.
    UnsupportedExtension {
        path: PathBuf,
        registered: Vec<&'static str>,
    },
    /// `Assets<T>` storage was missing from the resource set when the
    /// load completed. Caller forgot to insert it before driving a load.
    MissingAssetStorage(&'static str),
    /// `load_by_guid` was called for a [`Guid`] not registered in the
    /// project's [`AssetDatabase`]. The asset either has no `.meta`
    /// sidecar yet or was never scanned/loaded.
    UnknownGuid(Guid),
    /// Domain error returned by the loader itself (parser failure, etc.).
    Loader(Box<dyn std::error::Error + Send + Sync>),
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetError::Io(e) => write!(f, "asset I/O failed: {e}"),
            AssetError::NoLoaderForType(name) => {
                write!(f, "no AssetLoader registered for type `{name}`")
            }
            AssetError::UnsupportedExtension { path, registered } => write!(
                f,
                "loader does not support extension of {path:?}; registered = {registered:?}",
            ),
            AssetError::MissingAssetStorage(name) => {
                write!(f, "Assets<{name}> resource missing — insert it first")
            }
            AssetError::UnknownGuid(guid) => {
                write!(f, "GUID {guid} is not registered in AssetDatabase")
            }
            AssetError::Loader(e) => write!(f, "loader error: {e}"),
        }
    }
}

impl std::error::Error for AssetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AssetError::Io(e) => Some(e),
            AssetError::Loader(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<std::io::Error> for AssetError {
    fn from(err: std::io::Error) -> Self {
        AssetError::Io(err)
    }
}
