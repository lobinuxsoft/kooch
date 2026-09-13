//! Global mesh pool — concatenated meshlet/vertex/triangle storage shared by every registered
//! `MeshletMesh`.

mod descriptor;
mod gpu;
mod pool;
#[cfg(test)]
mod tests;

pub use descriptor::{MeshDescriptor, MeshHandle};
pub use gpu::GpuGlobalMeshPool;
pub use pool::{GlobalMeshPool, MeshBounds};
