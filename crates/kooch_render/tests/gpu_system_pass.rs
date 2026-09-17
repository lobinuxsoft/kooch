//! GPU integration test: a `GpuSystem` can open a render pass (#392).
//!
//! 🔴 The point of the encoder contract. While `dispatch` took a `wgpu::ComputePass`, a
//! post-process — a full-screen draw into a target — had nowhere to be recorded.

mod common;

use common::try_acquire_device;
use kooch_core::resource::Resources;
use kooch_core::system::GpuSystem;

const SIDE: u32 = 64;
const ROW_BYTES: u32 = SIDE * 4;

/// Clears a target to red through a render pass of its own.
struct ClearPass {
    view: wgpu::TextureView,
}

impl GpuSystem for ClearPass {
    fn init(&mut self, _: &wgpu::Device, _: &wgpu::Queue) {}

    fn prepare(&mut self, _: &wgpu::Device, _: &wgpu::Queue, _: &Resources) {}

    fn record(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::RED),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }

    fn name(&self) -> &str {
        "clear_pass"
    }

    fn is_initialized(&self) -> bool {
        true
    }
}

#[test]
fn a_gpu_system_can_draw() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gpu_system_target"),
        size: wgpu::Extent3d {
            width: SIDE,
            height: SIDE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let system = ClearPass {
        view: target.create_view(&wgpu::TextureViewDescriptor::default()),
    };
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gpu_system_readback"),
        size: (ROW_BYTES * SIDE) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&Default::default());
    system.record(&mut encoder);
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(ROW_BYTES),
                rows_per_image: Some(SIDE),
            },
        },
        wgpu::Extent3d {
            width: SIDE,
            height: SIDE,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    let pixels = readback.slice(..).get_mapped_range().to_vec();

    assert_eq!(&pixels[0..4], &[255, 0, 0, 255], "the pass did not clear");
}
