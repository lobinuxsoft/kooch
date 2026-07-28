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
    /// A reflection operation failed.
    Reflect(ReflectError),
    /// A field still held a live entity handle when the file was written.
    ///
    /// An index and a generation are reassigned on the next load, so a
    /// saved one points at whatever occupies that slot. Reaching here means
    /// the save path did not resolve the reference to a `PersistentId`, and
    /// refusing is the difference between a failed save and a scene that
    /// loads with its references pointing at arbitrary entities.
    UnresolvedReference {
        entity: String,
        component: String,
        field: String,
    },
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "failed to access scene file: {e}"),
            Self::Ron(e) => write!(f, "failed to serialize scene RON: {e}"),
            Self::RonSpanned(e) => write!(f, "failed to parse scene RON: {e}"),
            Self::Reflect(e) => write!(f, "reflection error: {e}"),
            Self::UnresolvedReference {
                entity,
                component,
                field,
            } => write!(
                f,
                "`{entity}`'s {component}.{field} still points at a live entity; \
                 the save path must resolve it to a PersistentId first",
            ),
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
            Self::UnresolvedReference { .. } => None,
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
