//! Global mesh pool — concatenated meshlet/vertex/triangle storage shared by every registered
//! `MeshletMesh`.

mod descriptor;
mod gpu;
mod gpu_owned;
mod pool;
mod relocate;
mod slots;
#[cfg(test)]
mod tests;

pub use descriptor::{MeshDescriptor, MeshHandle};
pub use gpu::GpuGlobalMeshPool;
pub use gpu_owned::GpuMeshSizes;
pub use pool::{GlobalMeshPool, MeshBounds};
