//! Test code for `pass`, in its own file.

use kooch_plugin_render::{PassFrame, RenderPass, TargetDesc};

use super::*;
use crate::gpu::TargetPool;

/// A device, or `None` on a runner without one.
fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
        flags: wgpu::InstanceFlags::default(),
        backend_options: wgpu::BackendOptions::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("plugin_pass_test_device"),
        ..Default::default()
    }))
    .ok()
}

/// Asks the pool for a target and clears it, which is the shape of every post-process.
#[derive(Default)]
struct Clear {
    ready: bool,
}

impl RenderPass for Clear {
    fn name(&self) -> &str {
        "clear"
    }

    fn init(&mut self, _: kooch_plugin_render::PassSetup<'_>) {
        self.ready = true;
    }

    fn record(&mut self, frame: PassFrame<'_>) {
        let target = frame.targets.acquire(
            "plugin_clear",
            TargetDesc::attachment((8, 8), wgpu::TextureFormat::Rgba8Unorm)
                .with_usage(wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC),
        );
        let Some(view) = frame.targets.view(target).cloned() else {
            return;
        };
        frame
            .encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("plugin_clear_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::GREEN),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
    }
}

/// Something that is not a pass at all.
#[test]
fn a_wrong_payload_is_refused() {
    assert!(PluginPass::from_erased(Box::new(7u32)).is_none());
}

/// The whole bridge: erased pass in, GPU system out, drawing into a target it asked the pool for.
#[test]
fn a_plugin_pass_draws_into_the_pool() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let erased: Box<dyn RenderPass> = Box::new(Clear::default());
    let mut pass = PluginPass::from_erased(Box::new(erased)).expect("a pass");
    assert!(!pass.is_initialized());
    pass.init(&device, &queue);
    assert!(pass.is_initialized());
    assert_eq!(GpuSystem::name(&pass), "clear");

    let mut targets = TargetPool::new(&device);
    let mut encoder = device.create_command_encoder(&Default::default());
    pass.record(
        Frame {
            device: &device,
            queue: &queue,
            targets: &mut targets,
        },
        &mut encoder,
    );

    // The pass asked the pool for its target, so the pool is what proves it drew.
    let target = targets.acquire(
        "unused",
        TargetDesc::attachment((8, 8), wgpu::TextureFormat::Rgba8Unorm),
    );
    assert_eq!(
        targets.created(),
        2,
        "the pass allocated one, this one is ours"
    );
    let _ = target;

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("plugin_pass_readback"),
        size: 256 * 8,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let drawn = targets.texture(kooch_plugin_render::TargetId(0)).unwrap();
    encoder.copy_texture_to_buffer(
        drawn.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(8),
            },
        },
        wgpu::Extent3d {
            width: 8,
            height: 8,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    let pixels = readback.slice(..).get_mapped_range().to_vec();

    assert_eq!(
        &pixels[0..4],
        &[0, 255, 0, 255],
        "the plugin's pass did not draw"
    );
}
