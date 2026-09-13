//! Writing meshes back out: GLB export, and simplification for colliders.

mod glb;
mod simplify;
#[cfg(test)]
mod tests;

pub use glb::{ExportError, to_glb, to_glb_parts};
pub use simplify::{SimplifyTarget, simplify};
