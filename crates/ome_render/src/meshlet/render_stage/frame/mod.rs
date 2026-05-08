//! Per-frame work for [`MeshletRenderStage`]: resize, GPU mesh cache
//! upkeep, ECS asset sync, and the cull → vbuf → deferred chain.
//!
//! Split into sibling files (`resize`, `assets`, `render`,
//! `render_r64`, `render_hi_z_2pass`) so each stays under the 400-LoC
//! ceiling and the per-frame phases are visible at a glance.
//!
//! `render()` itself is a thin dispatcher (prelude + path switch) that
//! routes the frame through `render_path_r64` (#493 atomic R64 vbuf)
//! when supported, or `render_path_hi_z_two_pass` (R32 + Hi-Z 2-pass)
//! as the legacy fallback. The path methods are private helpers
//! (`pub(super)` so the dispatcher in `render.rs` can call them across
//! sibling files).

mod assets;
mod render;
mod render_hi_z_2pass;
mod render_r64;
mod resize;

#[cfg(test)]
mod tests;
