//! `BlockMesh` — the authoring mesh the in-editor block tool edits.
//! A render `Mesh` duplicates positions per face and cannot be edited in place, so the tool edits
//! this and regenerates one. Only positions and CSR faces are serialised; adjacency is derived.

mod adjacency;
mod asset;
mod block;
mod block_mesh;
mod collider;
mod extrude;
mod generate;
mod pick;
mod plugin;
mod sync;

pub use adjacency::{Adjacency, NO_FACE};
pub use asset::{BLOCK_MESH_EXTENSION, BlockMeshLoader, BlockMeshParseError};
pub use block::{Block, block_components};
pub use block_mesh::BlockMesh;
pub use extrude::Extruded;
pub use pick::{Hit, Screen, edge_at, face_at, vertex_at};
pub use plugin::BlockPlugin;
pub use sync::{BuiltBlocks, sync_blocks};
