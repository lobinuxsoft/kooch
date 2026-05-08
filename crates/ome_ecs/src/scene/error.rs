use std::fmt;

use crate::reflect::ReflectError;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during scene save/load/sync.
#[derive(Debug)]
pub enum SceneError {
    /// File I/O error.
    Io(std::io::Error),
    /// RON serialization error.
    Ron(ron::Error),
    /// RON deserialization error (with span info).
    RonSpanned(ron::error::SpannedError),
    /// A component type referenced in the scene is not registered.
    UnknownComponent(String),
    /// A reflection operation failed.
    Reflect(ReflectError),
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "failed to access scene file: {e}"),
            Self::Ron(e) => write!(f, "failed to serialize scene RON: {e}"),
            Self::RonSpanned(e) => write!(f, "failed to parse scene RON: {e}"),
            Self::UnknownComponent(name) => write!(f, "unknown component type: {name}"),
            Self::Reflect(e) => write!(f, "reflection error: {e}"),
        }
    }
}

impl std::error::Error for SceneError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Ron(e) => Some(e),
            Self::RonSpanned(e) => Some(e),
            Self::Reflect(e) => Some(e),
            Self::UnknownComponent(_) => None,
        }
    }
}

impl From<std::io::Error> for SceneError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<ron::Error> for SceneError {
    fn from(e: ron::Error) -> Self {
        Self::Ron(e)
    }
}

impl From<ron::error::SpannedError> for SceneError {
    fn from(e: ron::error::SpannedError) -> Self {
        Self::RonSpanned(e)
    }
}

impl From<ReflectError> for SceneError {
    fn from(e: ReflectError) -> Self {
        Self::Reflect(e)
    }
}
