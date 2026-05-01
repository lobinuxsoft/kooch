//! Debug-only overlay counters for the GDF populate pass. Gated by
//! the `gdf-debug` feature — never compile this into a hot-path
//! build, the readback stalls the GPU pipeline.
//!
//! Three counters per the PR-3 plan:
//! - `voxels_written_last_frame` — total voxels populated by the
//!   most recent dispatch (deterministic; pinned by cascade dim, no
//!   readback required).
//! - `cascade_world_origin` — origin of the cascade as written to
//!   the uniform buffer on the most recent dispatch.
//! - `voxels_with_sdf_lt_zero` — count of voxels whose SDF is
//!   strictly negative, i.e. inside surface. Requires a CPU
//!   readback of the cascade texture; ProceduralCity at the origin
//!   should report a non-zero value within one frame.

use glam::Vec3;

use super::CASCADE_0_VOXELS_PER_AXIS;
use super::state::GdfState;

/// Snapshot of the GDF debug counters for one frame. Cheap to clone;
/// `Copy` so the editor's UI can keep a frame-stable copy without
/// borrowing back into `GdfState`.
#[derive(Copy, Clone, Debug, Default)]
pub struct GdfDebugCounters {
    pub voxels_written_last_frame: u64,
    pub cascade_world_origin: Vec3,
    pub voxels_with_sdf_lt_zero: u64,
}

impl GdfState {
    /// Read back the cascade-0 texture to the CPU and tally the debug
    /// counters. Stalls the GPU pipeline; **only call from debug
    /// overlay code or tests**.
    ///
    /// Returns `None` when the cascade has not yet been populated
    /// (the descriptor's `voxel_count_per_axis` is zero by default).
    pub fn debug_readback_counters(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Option<GdfDebugCounters> {
        let descriptor = self.last_descriptor();
        if descriptor.voxel_count_per_axis == 0 {
            return None;
        }
        let voxels = readback_cascade(device, queue, self);
        let inside = voxels.iter().filter(|v| **v < 0.0).count() as u64;
        let total = (CASCADE_0_VOXELS_PER_AXIS as u64).pow(3);
        Some(GdfDebugCounters {
            voxels_written_last_frame: total,
            cascade_world_origin: Vec3::from_array(descriptor.world_origin),
            voxels_with_sdf_lt_zero: inside,
        })
    }
}

/// Internal mirror of `tests/common/gdf::readback_cascade` — kept
/// here so the production library does not pull in a test-only
/// helper crate. Same row-stride alignment guard.
fn readback_cascade(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    state: &GdfState,
) -> Vec<f32> {
    let n = CASCADE_0_VOXELS_PER_AXIS;
    let bytes_per_pixel = 4u32;
    const _: () = assert!(
        (CASCADE_0_VOXELS_PER_AXIS * 4) % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT == 0,
        "cascade row stride must align to 256 B for buffer readback"
    );
    let bytes_per_row = n * bytes_per_pixel;
    let rows_per_image = n;
    let total_bytes = (bytes_per_row * rows_per_image * n) as u64;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_render::gdf::debug_readback_staging"),
        size: total_bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("ome_render::gdf::debug_readback_encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: state.cascade_texture(),
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(rows_per_image),
            },
        },
        wgpu::Extent3d { width: n, height: n, depth_or_array_layers: n },
    );
    queue.submit(Some(encoder.finish()));
    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        tx.send(r).ok();
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("device poll");
    rx.recv().expect("map_async sender").expect("map_async result");
    let data = slice.get_mapped_range();
    let out: Vec<f32> = bytemuck::cast_slice::<u8, f32>(&data).to_vec();
    drop(data);
    staging.unmap();
    out
}
