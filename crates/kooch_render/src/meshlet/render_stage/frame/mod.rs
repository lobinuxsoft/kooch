//! Per-frame work for [`MeshletRenderStage`]: resize, GPU mesh cache upkeep, ECS asset sync, and
//! the cull → vbuf → deferred chain.

mod assets;
mod pages;
mod render;
mod render_hi_z_2pass;
mod render_r64;
mod resize;
mod shadows;
mod trim;
pub(in crate::meshlet::render_stage) use shadows::ClassicAlloc;

#[cfg(test)]
mod tests;
