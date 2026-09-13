//! Robust Contrast Adaptive Sharpening (#481, step 5).

use bytemuck::{Pod, Zeroable};

use crate::meshlet::deferred::DEFERRED_COLOR_FORMAT;

const SHADER_SOURCE: &str = include_str!("../../../shaders/rcas.wgsl");

#[cfg(test)]
mod tests;

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
struct SharpenUbo {
    sharpness: f32,
    _pad: [f32; 3],
}

/// The author's amount, as the shader wants it.
pub fn sharpness_of(percent: u32) -> f32 {
    percent.min(100) as f32 / 100.0
}

pub(super) struct Sharpen {
    pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    ubo: wgpu::Buffer,
    /// What the tonemap writes into while this pass runs. Output
    /// resolution: sharpening is the last thing that happens to the
    /// image, so it works at the size the image is presented at.
    ldr: wgpu::Texture,
    ldr_view: wgpu::TextureView,
}

impl Sharpen {
    pub(super) fn new(device: &wgpu::Device, output: (u32, u32)) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rcas_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rcas_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        // Not filterable, and it never samples: the five
                        // taps are integer loads of exact neighbours.
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rcas_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rcas_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(DEFERRED_COLOR_FORMAT.into())],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let ubo = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rcas_ubo"),
            size: std::mem::size_of::<SharpenUbo>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (ldr, ldr_view) = create_ldr(device, output);
        Self {
            pipeline,
            bgl,
            ubo,
            ldr,
            ldr_view,
        }
    }

    pub(super) fn resize(&mut self, device: &wgpu::Device, output: (u32, u32)) {
        let (ldr, ldr_view) = create_ldr(device, output);
        self.ldr = ldr;
        self.ldr_view = ldr_view;
    }

    /// Where the tonemap writes when this pass is going to run.
    pub(super) fn input_view(&self) -> &wgpu::TextureView {
        &self.ldr_view
    }

    /// Sharpens [`Self::input_view`] onto `target`.
    pub(super) fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        percent: u32,
    ) {
        queue.write_buffer(
            &self.ubo,
            0,
            bytemuck::bytes_of(&SharpenUbo {
                sharpness: sharpness_of(percent),
                _pad: [0.0; 3],
            }),
        );

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rcas_bind_group"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.ldr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.ubo.as_entire_binding(),
                },
            ],
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("rcas_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                // Every pixel is written, so the load discards.
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn create_ldr(device: &wgpu::Device, output: (u32, u32)) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sharpen_ldr"),
        size: wgpu::Extent3d {
            width: output.0.max(1),
            height: output.1.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEFERRED_COLOR_FORMAT,
        // RENDER_ATTACHMENT for the tonemap writing it, TEXTURE_BINDING
        // for this pass reading it back, COPY_SRC so a test can compare
        // the image before sharpening with the one after.
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}
