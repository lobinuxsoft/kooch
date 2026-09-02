//! Writing meshes back out: GLB export, and simplification for colliders.
//!
//! The engine has always been able to read `.glb` and never to write one.
//! That was fine while every mesh came from an artist, and stops being
//! fine the moment the engine generates geometry itself — a baked
//! primitive (#573) or a simplified collision mesh (#137) has to land
//! somewhere an artist can open.

mod glb;
mod simplify;
#[cfg(test)]
mod tests;

pub use glb::{ExportError, to_glb, to_glb_parts};
pub use simplify::{SimplifyTarget, simplify};
