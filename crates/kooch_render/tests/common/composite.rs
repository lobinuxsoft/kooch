//! Composing the stage onto a target the way a frame does: whatever was drawn first (the sky, a
//! clear), then the stage's colour and depth blitted over it, then any overlay views.

use kooch_render::VIEWPORT_DEPTH_FORMAT;
use kooch_render::meshlet::{MeshletBlit, ViewId};

use super::lit_scene::{Rig, SIZE};

/// One composed image, colour and depth: the depth is where a wrong composite shows up, because a
/// blit that writes its empty pixels leaves the right colour and no geometry under it.
pub struct Frame {
    pub color: Vec<u8>,
    pub depth: Vec<f32>,
}

/// The primary view blitted onto a target cleared to `sky`, then `overlays` over it, read back —
/// the way the frame composes the stage over whatever was drawn before it.
pub fn composite(rig: &Rig, overlays: &[ViewId], sky: wgpu::Color) -> Frame {
    let target = rig.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("camera_stack_target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let depth = rig.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("camera_stack_depth"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: VIEWPORT_DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let depth_view = depth.create_view(&Default::default());
    let blit = MeshletBlit::new(
        &rig.device,
        wgpu::TextureFormat::Rgba8Unorm,
        VIEWPORT_DEPTH_FORMAT,
    );

    let mut encoder = rig
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("camera_stack_encoder"),
        });
    // The clear the base camera owns: an overlay brings none of this.
    drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("camera_stack_clear"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(sky),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &depth_view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(0.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    }));
    blit.blit(
        &rig.device,
        &mut encoder,
        rig.stage.color_view(),
        rig.stage.depth_sample_view(),
        &view,
        &depth_view,
    );
    for overlay in overlays {
        blit.blit(
            &rig.device,
            &mut encoder,
            rig.stage
                .view_color_view(*overlay)
                .expect("the overlay view is live"),
            rig.stage
                .view_depth_sample(*overlay)
                .expect("the overlay view is live"),
            &view,
            &depth_view,
        );
    }
    rig.queue.submit(Some(encoder.finish()));
    Frame {
        color: super::read_rgba8(&rig.device, &rig.queue, &target),
        depth: read_depth(rig, &depth),
    }
}

/// A `Depth32Float` attachment, tightly packed.
fn read_depth(rig: &Rig, texture: &wgpu::Texture) -> Vec<f32> {
    let padded = (SIZE * 4).div_ceil(256) * 256;
    let staging = rig.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("camera_stack_depth_readback"),
        size: u64::from(padded * SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = rig
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("camera_stack_depth_copy"),
        });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::DepthOnly,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    rig.queue.submit(Some(encoder.finish()));
    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        tx.send(r).expect("the send lands")
    });
    let _ = rig.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
    rx.recv()
        .expect("the map answers")
        .expect("the map succeeds");
    let bytes = slice.get_mapped_range().to_vec();
    let mut depth = Vec::with_capacity((SIZE * SIZE) as usize);
    for row in 0..SIZE {
        let at = (row * padded) as usize;
        for x in 0..SIZE as usize {
            let word = &bytes[at + x * 4..at + x * 4 + 4];
            depth.push(f32::from_le_bytes(word.try_into().expect("four bytes")));
        }
    }
    depth
}
