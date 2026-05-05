//! Error type returned by every entry point in this builder module.

/// Errors raised while building a meshlet mesh.
#[derive(Debug)]
pub enum MeshletBuildError {
    /// Source mesh had no triangles.
    EmptyMesh,
    /// `meshopt` rejected the vertex layout (stride mismatch, etc.).
    VertexAdapter(meshopt::Error),
}

impl std::fmt::Display for MeshletBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyMesh => write!(f, "cannot build meshlets from a mesh with zero triangles"),
            Self::VertexAdapter(e) => write!(f, "meshopt vertex adapter failed: {e}"),
        }
    }
}

impl std::error::Error for MeshletBuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::VertexAdapter(e) => Some(e),
            _ => None,
        }
    }
}

impl From<meshopt::Error> for MeshletBuildError {
    fn from(e: meshopt::Error) -> Self {
        Self::VertexAdapter(e)
    }
}
