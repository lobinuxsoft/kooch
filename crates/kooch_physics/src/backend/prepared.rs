//! A shape the backend already built, so a worker thread pays for it instead of the frame that
//! adds it (#1949: a trimesh with fixed internal edges costs milliseconds to build).

use std::any::Any;
use std::sync::Arc;

/// Opaque to everything but the backend that made it; see [`crate::RapierBackend::prepare`].
#[derive(Clone)]
pub struct PreparedShape(pub(crate) Arc<dyn Any + Send + Sync>);

impl std::fmt::Debug for PreparedShape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PreparedShape")
    }
}
