//! A masked material's cut over its uv square, read back to the CPU (#452).
//!
//! 🔴 The readback blocks. It belongs to the asset step and runs once per (mesh, material, values)
//! pair, never in a frame's path: 64 KiB at [`TRIM_SIDE`](super::TRIM_SIDE).

use kooch_core::buffer::StagingBuffer;

use crate::material::{MaterialPipeline, MaterialTexturePool};
use crate::meshlet::MATERIAL_SURFACE_PRELUDE;
use crate::shadow::alpha::{BAKE_FRAME, BakeScreen};

/// `side`² texels of what `slot`'s surface answers over its uv square: a masked material's cut, 255
/// where a fragment survives and 0 where it is discarded, or a transparent one's own alpha. Public
/// because a surface asked over its uv, off screen, is the only way to test one by its numbers.
pub fn mask(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    materials: &MaterialPipeline,
    slot: u32,
    side: u32,
) -> Option<Vec<u8>> {
    let (_, surface) = materials.slot_surface(slot)?;
    let source = [
        MATERIAL_SURFACE_PRELUDE,
        &surface.params_wgsl,
        &surface.source,
        BAKE_FRAME,
    ]
    .join("\n");
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("alpha_trim_bake"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });

    let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let uniform = |binding| entry(binding, wgpu::BufferBindingType::Uniform);
    let storage = |binding| {
        entry(
            binding,
            wgpu::BufferBindingType::Storage { read_only: true },
        )
    };
    let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries,
        })
    };
    let frame_bgl = layout("alpha_trim_frame_bgl", &[uniform(0), uniform(1)]);
    let materials_bgl = layout("alpha_trim_materials_bgl", &[storage(0), storage(1)]);
    // Groups 1 and 3 belong to the mesh pool and the visible list, which a bake has neither of.
    let empty_bgl = layout("alpha_trim_empty_bgl", &[]);
    let textures_bgl = MaterialTexturePool::bind_group_layout(device);
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("alpha_trim_bake_layout"),
        bind_group_layouts: &[
            Some(&frame_bgl),
            Some(&empty_bgl),
            Some(&materials_bgl),
            Some(&empty_bgl),
            Some(&textures_bgl),
        ],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("alpha_trim_bake"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_bake"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_bake"),
            targets: &[Some(wgpu::TextureFormat::R8Unorm.into())],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let screen = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("alpha_trim_screen"),
        size: std::mem::size_of::<BakeScreen>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let inti = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("alpha_trim_inti"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(
        &screen,
        0,
        bytemuck::bytes_of(&BakeScreen::new(slot, 0.0, side as f32)),
    );
    queue.write_buffer(&inti, 0, &[0u8; 16]);
    let binding = |binding, resource| wgpu::BindGroupEntry { binding, resource };
    let frame_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("alpha_trim_frame_bg"),
        layout: &frame_bgl,
        entries: &[
            binding(0, screen.as_entire_binding()),
            binding(1, inti.as_entire_binding()),
        ],
    });
    let materials_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("alpha_trim_materials_bg"),
        layout: &materials_bgl,
        entries: &[
            binding(0, materials.pool().buffer().as_entire_binding()),
            binding(1, materials.pool().values().as_entire_binding()),
        ],
    });
    let empty_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("alpha_trim_empty_bg"),
        layout: &empty_bgl,
        entries: &[],
    });
    let textures_bg = materials
        .texture_pool()
        .material_bind_group(device, &materials.slot_texture_refs(slot));

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("alpha_trim_mask"),
        size: wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let staging = StagingBuffer::new(device, (side * side) as u64);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("alpha_trim_bake"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("alpha_trim_bake"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &frame_bg, &[]);
        pass.set_bind_group(1, &empty_bg, &[]);
        pass.set_bind_group(2, &materials_bg, &[]);
        pass.set_bind_group(3, &empty_bg, &[]);
        pass.set_bind_group(4, &textures_bg, &[]);
        pass.draw(0..3, 0..1);
    }
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: staging.buffer(),
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(side),
                rows_per_image: Some(side),
            },
        },
        wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));
    Some(staging.read_back::<u8>(device))
}
