//! Global mesh pool — concatenated meshlet/vertex/triangle storage
//! shared by every registered `MeshletMesh`. Phase 1.E.1b: lets the
//! scene-wide cull dispatch enumerate meshlets across **different**
//! meshes via a `mesh_id` indirection, instead of being locked to a
//! single registered mesh as in 1.E.1.
//!
//! # Layout
//!
//! Each registered mesh appends to four flat CPU arrays:
//! - `meshlets`: `Vec<MeshletDescriptor>` — per-mesh meshlet metadata
//!   (offsets re-based into pool-global coordinates).
//! - `vertices`: `Vec<MeshVertex>` — concatenated vertex pools.
//! - `meshlet_vertices`: `Vec<u32>` — concatenated meshlet→pool
//!   indices, rebased to point into this pool's `vertices`.
//! - `meshlet_triangles`: `Vec<u8>` — concatenated raw triangle bytes
//!   (3 bytes per triangle, padded to 4-byte boundaries between
//!   meshes so future GPU `array<u32>` reads stay aligned).
//!
//! A parallel `mesh_descriptors: Vec<MeshDescriptor>` carries each
//! mesh's `(first_meshlet, meshlet_count, vertex_offset,
//! meshlet_vertex_offset, meshlet_triangle_offset)`. The GPU reads
//! this descriptor at `inst.mesh_id` to redirect every per-meshlet
//! lookup.

mod descriptor;
mod gpu;
mod pool;
#[cfg(test)]
mod tests;

pub use descriptor::{MeshDescriptor, MeshHandle};
pub use gpu::GpuGlobalMeshPool;
pub use pool::GlobalMeshPool;
