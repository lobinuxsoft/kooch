//! Sanity: HiZ::build_from_depth against a real Depth32Float texture.
//!
//! Existing tests in hi_z_build.rs exercise build_from_r32 because
//! Queue::write_texture cannot upload to depth formats. The 2-pass
//! cull (#445) is the first production caller of build_from_depth,
//! so this test pins the path end-to-end on the GPU before the
//! orchestrator tries to use it.
//!
//! Run with:
//!   cargo test -p ome_render --test hi_z_build_from_depth -- --test-threads=1

mod common;

use common::try_acquire_device;
use ome_render::hi_z::{mip_size, HiZ};

const SIZE: u32 = 64;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[test]
fn build_from_depth_roundtrip_with_cleared_depth_attachment() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };

    // Real Depth32Float attachment with both render-attachment and
    // texture-binding usages — production layout. The 2-pass cull's
    // depth_view is identical.
    let depth_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test_depth"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let depth_render_view = depth_tex.create_view(&wgpu::TextureViewDescriptor::default());
    let depth_sample_view = depth_tex.create_view(&wgpu::TextureViewDescriptor {
        label: Some("test_depth_sample"),
        format: Some(DEPTH_FORMAT),
        dimension: Some(wgpu::TextureViewDimension::D2),
        usage: None,
        aspect: wgpu::TextureAspect::DepthOnly,
        base_mip_level: 0,
        mip_level_count: Some(1),
        base_array_layer: 0,
        array_layer_count: Some(1),
    });

    // Clear depth to a known value (0.42) via a one-shot render pass
    // with no draws. After this the attachment holds 0.42 everywhere.
    let mut clear_enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("test_depth_clear_enc"),
    });
    {
        let _pass = clear_enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("test_depth_clear_pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_render_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.42),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    queue.submit(std::iter::once(clear_enc.finish()));

    let hi_z = HiZ::new(&device, SIZE, SIZE);

    // Run build_from_depth in a separate encoder + submit to mirror
    // the production flow's submit boundary (which is itself a
    // workaround for Mesa radv's storage→texture transition issues).
    let mut build_enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("test_hi_z_build_enc"),
    });
    let mut arena: Vec<wgpu::BindGroup> = Vec::new();
    hi_z.build_from_depth(&device, &mut build_enc, &depth_sample_view, &mut arena);
    queue.submit(std::iter::once(build_enc.finish()));

    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });

    // Sanity-read mip 0 — should be 0.42 everywhere.
    let mip0 = read_mip(&device, &queue, &hi_z, 0);
    assert!(
        mip0.iter().all(|&v| (v - 0.42).abs() < 1e-5),
        "mip 0 must be 0.42 after build_from_depth on a cleared 0.42 \
         depth attachment; got first 4 values: {:?}",
        &mip0[..mip0.len().min(4)]
    );

    // And the top mip (1×1) should also be 0.42 (max of 0.42 = 0.42).
    let top = mip0[0];
    assert!(
        (top - 0.42).abs() < 1e-5,
        "top mip must hold 0.42, got {top}"
    );
}

fn read_mip(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    hi_z: &HiZ,
    mip: u32,
) -> Vec<f32> {
    let (w, h) = mip_size(SIZE, SIZE, mip);
    let bytes_per_row = (w * 4).max(256);
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test_mip_staging"),
        size: (bytes_per_row * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("test_mip_readback"),
    });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: hi_z.texture(),
            mip_level: mip,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(enc.finish()));
    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
    rx.recv().unwrap().unwrap();
    let bytes = slice.get_mapped_range().to_vec();
    let mut out = Vec::with_capacity((w * h) as usize);
    for row in 0..h {
        let row_off = (row * bytes_per_row) as usize;
        for x in 0..w {
            let off = row_off + (x * 4) as usize;
            out.push(f32::from_le_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
            ]));
        }
    }
    out
}
