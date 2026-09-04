//! `BlockMesh` — the authoring mesh the in-editor block tool edits.
//!
//! [`kooch_render::Mesh`] is a triangle soup with positions duplicated
//! per face, because each face carries its own normal. That layout is
//! what the GPU wants and what the meshlet builder eats, but it cannot
//! be edited in place: moving one corner means finding every copy of
//! it. So the tool edits a `BlockMesh` and *generates* a `Mesh` from it
//! on every change — the same split ProBuilder draws between its
//! authoring mesh and Unity's render mesh.
//!
//! # What is stored, and what is not
//!
//! Only canonical data is serialised: shared [`positions`] plus the
//! faces that index them, in CSR form. Adjacency (edges, and which two
//! faces meet along one) is **derived**, never stored — deriving it
//! costs one pass and keeps it out of the file, so the topology
//! structures the operators want can change without breaking a single
//! saved level.
//!
//! [`positions`]: BlockMesh::positions
//!
//! # Two outputs, on purpose
//!
//! - [`BlockMesh::to_mesh`] splits positions per face, so faces shade
//!   flat. That is what gets rendered.
//! - [`BlockMesh::triangles`] indexes the shared positions, welded. That
//!   is what the collider gets — a physics trimesh wants corners that
//!   coincide to *be* one corner.

mod asset;
mod block_mesh;
mod generate;

pub use asset::{BlockMeshLoader, BlockMeshParseError};
pub use block_mesh::BlockMesh;
