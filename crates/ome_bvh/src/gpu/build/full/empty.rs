//! [`empty_build`] — placeholder constructor for the `n == 0` case.

use std::sync::Arc;

use crate::gpu::builder::BvhGpuBuilder;

use super::super::lifecycle::MapState;
use super::build::BvhGpuBuild;

/// Construct a placeholder `BvhGpuBuild` for the `n == 0` case. No GPU
/// dispatches and no submission — `submission_index = None` makes
/// `BvhGpuBuild::poll` return `Some(Ok(Bvh::empty()))` immediately.
/// Staging buffers are minimal placeholders that are never mapped.
pub(super) fn empty_build<T: Copy>(
    builder: &BvhGpuBuilder,
    device: &wgpu::Device,
) -> BvhGpuBuild<T> {
    let placeholder = |label: &str| -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    };
    BvhGpuBuild {
        n: 0,
        submission_index: None,
        nodes_staging: placeholder("ome_bvh::build_gpu_empty_nodes_staging"),
        indices_staging: placeholder("ome_bvh::build_gpu_empty_indices_staging"),
        nodes_buffer: builder.lbvh_buffers.nodes_buffer.clone(),
        map_state: Arc::new(MapState::default()),
        payloads: Vec::new(),
        consumed: false,
        done_staging: None,
        debug_input_aabbs: None,
    }
}
