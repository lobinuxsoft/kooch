//! Drawing the UI over the finished frame (#96): instanced draws of [`UiQuad`]s, one per stretch
//! of plain or same-shader quads, after the post-process, into the image that is presented.

use std::ops::Range;

use kooch_ecs::{UiCurve, UiDrawList, UiMask, UiMirror, UiQuad};

use crate::material::MAX_PARAM_SCALARS;

/// What the pass keeps between frames: pipeline, glyph atlas texture, instance buffer.
pub struct UiPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    screen: wgpu::Buffer,
    atlas: wgpu::Texture,
    atlas_side: u32,
    bind: wgpu::BindGroup,
    instances: wgpu::Buffer,
    capacity: u64,
    pipeline_layout: wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    materials: UiMaterials,
    /// [`UiMaterials::values`] on the GPU.
    values: wgpu::Buffer,
    values_capacity: u64,
    /// [`UiDrawList::masks`] on the GPU.
    masks: wgpu::Buffer,
    masks_capacity: u64,
    /// [`UiDrawList::curves`] on the GPU.
    curves: wgpu::Buffer,
    curves_capacity: u64,
    /// [`UiDrawList::mirrors`] on the GPU.
    mirrors: wgpu::Buffer,
    mirrors_capacity: u64,
    started: std::time::Instant,
}

mod materials;

use materials::UiMaterials;
pub use materials::validate_ui;

const ATTRIBUTES: [wgpu::VertexAttribute; 12] = wgpu::vertex_attr_array![
    0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4,
    5 => Float32x2, 6 => Uint32x2, 7 => Uint32x2, 8 => Uint32x2, 9 => Uint32, 10 => Uint32,
    11 => Uint32,
];

/// `UI_CURVE_STRIPS` in `ui.wgsl`, as vertices of a triangle strip.
const CURVED_VERTICES: u32 = (64 + 1) * 2;

impl UiPass {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ui"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/ui.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    // The fragment too: `time` lives here.
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui_layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = pipeline(device, &pipeline_layout, format, &module, "fs_ui");
        let screen = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui_screen"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let values_capacity = 16;
        let values = values_buffer(device, values_capacity);
        let masks_capacity = 16;
        let masks = masks_buffer(device, masks_capacity);
        let curves_capacity = 4;
        let curves = curves_buffer(device, curves_capacity);
        let mirrors_capacity = 4;
        let mirrors = mirrors_buffer(device, mirrors_capacity);
        let buffers = Buffers {
            screen: &screen,
            values: &values,
            masks: &masks,
            curves: &curves,
            mirrors: &mirrors,
        };
        let (atlas, bind) = atlas_and_bind(device, &layout, buffers, 1);
        let capacity = 256;
        Self {
            pipeline,
            layout,
            screen,
            atlas,
            atlas_side: 1,
            bind,
            instances: instance_buffer(device, capacity),
            capacity,
            pipeline_layout,
            format,
            materials: UiMaterials::default(),
            values,
            values_capacity,
            masks,
            masks_capacity,
            curves,
            curves_capacity,
            mirrors,
            mirrors_capacity,
            started: std::time::Instant::now(),
        }
    }

    /// Uploads what changed and draws `list` over `target`, or over transparent black when `clear`.
    /// Takes the atlas upload, so it is written once.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size: (u32, u32),
        list: &mut UiDrawList,
        clear: bool,
    ) {
        if list.atlas_side != self.atlas_side && list.atlas_side > 0 {
            let buffers = self.buffers();
            let (atlas, bind) = atlas_and_bind(device, &self.layout, buffers, list.atlas_side);
            self.atlas = atlas;
            self.bind = bind;
            self.atlas_side = list.atlas_side;
        }
        if let Some(upload) = list.atlas_upload.take() {
            let [x0, y0, x1, y1] = upload.rect;
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: x0, y: y0, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &upload.texels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some((x1 - x0) * 4),
                    rows_per_image: Some(y1 - y0),
                },
                wgpu::Extent3d {
                    width: x1 - x0,
                    height: y1 - y0,
                    depth_or_array_layers: 1,
                },
            );
        }
        if list.quads.is_empty() && !clear {
            return;
        }
        let count = list.quads.len() as u64;
        if count > self.capacity {
            self.capacity = count.next_power_of_two();
            self.instances = instance_buffer(device, self.capacity);
        }
        queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&list.quads));
        self.materials
            .prepare(device, &self.pipeline_layout, self.format, list);
        self.upload_values(device, queue);
        self.upload_masks(device, queue, &list.masks);
        self.upload_curves(device, queue, &list.curves);
        self.upload_mirrors(device, queue, &list.mirrors);
        let time = self.started.elapsed().as_secs_f32();
        queue.write_buffer(
            &self.screen,
            0,
            bytemuck::cast_slice(&[size.0 as f32, size.1 as f32, time, 0.0]),
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: match clear {
                        true => wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        false => wgpu::LoadOp::Load,
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if count == 0 {
            return;
        }
        pass.set_bind_group(0, &self.bind, &[]);
        pass.set_vertex_buffer(
            0,
            self.instances.slice(..count * size_of::<UiQuad>() as u64),
        );
        // In order, plain quads between the runs: a material sits where the tree put it.
        let mut drawn = 0;
        for run in &self.materials.runs {
            let Some(material) = self.materials.pipeline(run.shader) else {
                continue;
            };
            if run.quads.start > drawn {
                pass.set_pipeline(&self.pipeline);
                draw_quads(&mut pass, &list.quads, drawn..run.quads.start);
            }
            pass.set_pipeline(material);
            draw_quads(&mut pass, &list.quads, run.quads.clone());
            drawn = run.quads.end;
        }
        if drawn < count as u32 {
            pass.set_pipeline(&self.pipeline);
            draw_quads(&mut pass, &list.quads, drawn..count as u32);
        }
    }

    fn buffers(&self) -> Buffers<'_> {
        Buffers {
            screen: &self.screen,
            values: &self.values,
            masks: &self.masks,
            curves: &self.curves,
            mirrors: &self.mirrors,
        }
    }

    /// Growing the buffer rebuilds the bind group, which names it.
    fn upload_values(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let values = &self.materials.values;
        if values.is_empty() {
            return;
        }
        let blocks = (values.len() / MAX_PARAM_SCALARS as usize) as u64;
        if blocks > self.values_capacity {
            self.values_capacity = blocks.next_power_of_two();
            self.values = values_buffer(device, self.values_capacity);
            self.bind = bind_group(device, &self.layout, self.buffers(), &self.atlas);
        }
        queue.write_buffer(&self.values, 0, bytemuck::cast_slice(values));
    }

    fn upload_masks(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, masks: &[UiMask]) {
        if masks.is_empty() {
            return;
        }
        if masks.len() as u64 > self.masks_capacity {
            self.masks_capacity = (masks.len() as u64).next_power_of_two();
            self.masks = masks_buffer(device, self.masks_capacity);
            self.bind = bind_group(device, &self.layout, self.buffers(), &self.atlas);
        }
        queue.write_buffer(&self.masks, 0, bytemuck::cast_slice(masks));
    }

    fn upload_curves(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, curves: &[UiCurve]) {
        if curves.is_empty() {
            return;
        }
        if curves.len() as u64 > self.curves_capacity {
            self.curves_capacity = (curves.len() as u64).next_power_of_two();
            self.curves = curves_buffer(device, self.curves_capacity);
            self.bind = bind_group(device, &self.layout, self.buffers(), &self.atlas);
        }
        queue.write_buffer(&self.curves, 0, bytemuck::cast_slice(curves));
    }

    fn upload_mirrors(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, mirrors: &[UiMirror]) {
        if mirrors.is_empty() {
            return;
        }
        if mirrors.len() as u64 > self.mirrors_capacity {
            self.mirrors_capacity = (mirrors.len() as u64).next_power_of_two();
            self.mirrors = mirrors_buffer(device, self.mirrors_capacity);
            self.bind = bind_group(device, &self.layout, self.buffers(), &self.atlas);
        }
        queue.write_buffer(&self.mirrors, 0, bytemuck::cast_slice(mirrors));
    }
}

/// `range`, a draw per stretch of straight or curved quads: only a curved one pays for its strips.
fn draw_quads(pass: &mut wgpu::RenderPass<'_>, quads: &[UiQuad], range: Range<u32>) {
    let mut at = range.start;
    while at < range.end {
        let curved = quads[at as usize].curve != 0;
        let end = (at..range.end)
            .find(|&next| (quads[next as usize].curve != 0) != curved)
            .unwrap_or(range.end);
        let vertices = if curved { CURVED_VERTICES } else { 4 };
        pass.draw(0..vertices, at..end);
        at = end;
    }
}

/// What the bind group names besides the atlas.
#[derive(Clone, Copy)]
struct Buffers<'a> {
    screen: &'a wgpu::Buffer,
    values: &'a wgpu::Buffer,
    masks: &'a wgpu::Buffer,
    curves: &'a wgpu::Buffer,
    mirrors: &'a wgpu::Buffer,
}

/// The pipeline every UI draw shares but for its fragment entry.
fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    module: &wgpu::ShaderModule,
    fragment: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("ui"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_ui"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<UiQuad>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &ATTRIBUTES,
            })],
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// Draws the frame's [`UiDrawList`](kooch_ecs::UiDrawList), building the pass on first use.
pub fn draw_frame(
    gpu: &kooch_core::gpu::GpuContext,
    encoder: &mut wgpu::CommandEncoder,
    resources: &mut kooch_core::resource::Resources,
    target: &wgpu::TextureView,
    size: (u32, u32),
) {
    let Some(mut list) = resources.remove::<kooch_ecs::UiDrawList>() else {
        return;
    };
    let mut pass = resources
        .remove::<UiPass>()
        .unwrap_or_else(|| UiPass::new(gpu.device(), gpu.format()));
    pass.draw(
        gpu.device(),
        gpu.queue(),
        encoder,
        target,
        size,
        &mut list,
        false,
    );
    resources.insert(pass);
    resources.insert(list);
}

fn instance_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ui_instances"),
        size: capacity * size_of::<UiQuad>() as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn values_buffer(device: &wgpu::Device, blocks: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ui_values"),
        size: blocks * MAX_PARAM_SCALARS as u64 * size_of::<f32>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn masks_buffer(device: &wgpu::Device, count: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ui_masks"),
        size: count * size_of::<UiMask>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn curves_buffer(device: &wgpu::Device, count: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ui_curves"),
        size: count * size_of::<UiCurve>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn mirrors_buffer(device: &wgpu::Device, count: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ui_mirrors"),
        size: count * size_of::<UiMirror>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn atlas_and_bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffers: Buffers<'_>,
    side: u32,
) -> (wgpu::Texture, wgpu::BindGroup) {
    let atlas = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ui_atlas"),
        size: wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // Not sRGB: the colours are display-space already, like the surface they land on.
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let bind = bind_group(device, layout, buffers, &atlas);
    (atlas, bind)
}

fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffers: Buffers<'_>,
    atlas: &wgpu::Texture,
) -> wgpu::BindGroup {
    let view = atlas.create_view(&Default::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("ui_images"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ui_bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers.screen.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: buffers.values.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: buffers.masks.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: buffers.curves.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: buffers.mirrors.as_entire_binding(),
            },
        ],
    })
}

#[cfg(test)]
mod tests;
