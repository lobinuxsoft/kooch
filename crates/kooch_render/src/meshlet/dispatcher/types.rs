//! POD types uploaded to the cull compute pipelines.

use bytemuck::{Pod, Zeroable};
use glam::Mat4;

/// Indirect draw arguments laid out for `wgpu::RenderPass::draw_indirect`.
///
/// `vertex_count` is fixed at pipeline creation (one expanded triangle
/// fan per meshlet, see `MeshletCull::vertex_count_per_instance`).
/// `instance_count` is the only per-frame dynamic field — mirrored
/// from the cull shader's atomic counter via
/// `encoder.copy_buffer_to_buffer`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
pub struct DrawIndirectArgs {
    pub vertex_count: u32,
    pub instance_count: u32,
    pub first_vertex: u32,
    pub first_instance: u32,
}

/// Per-frame uniform consumed by `cs_cull_hi_z` — the camera matrices
/// the shader needs to project a meshlet's bounding sphere onto the
/// Hi-Z pyramid and the pyramid's own dimensions / mip count.
///
/// Layout matches the WGSL `HiZParams` struct exactly. 80 bytes total.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
pub struct HiZTestParams {
    pub view_proj: [[f32; 4]; 4],
    pub hi_z_size: [f32; 2],
    pub hi_z_mip_count: u32,
    pub _pad0: u32,
}

impl HiZTestParams {
    pub fn new(view_proj: Mat4, hi_z_width: u32, hi_z_height: u32, mip_count: u32) -> Self {
        Self {
            view_proj: view_proj.to_cols_array_2d(),
            hi_z_size: [hi_z_width as f32, hi_z_height as f32],
            hi_z_mip_count: mip_count,
            _pad0: 0,
        }
    }
}
