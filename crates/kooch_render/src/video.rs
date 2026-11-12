//! A video frame converted on the GPU (#1793): its YUV planes uploaded as dav1d left them, 12 MB a
//! 4K frame against 33 MB of RGBA, and drawn as RGB straight into the texture that shows it.

use kooch_video::Picture;

/// One video's planes on the GPU, rewritten in place while its size holds.
pub struct VideoPlanes {
    planes: [wgpu::Texture; 3],
    params: wgpu::Buffer,
    bind: wgpu::BindGroup,
    full: bool,
}

/// Where a frame is drawn: the target's pixel of its top-left, and whether a border texel
/// surrounds it there.
#[derive(Clone, Copy)]
pub struct VideoTarget<'a> {
    pub texture: &'a wgpu::Texture,
    pub origin: [u32; 2],
    pub border: bool,
    /// The texels the frame fills, border aside; `None` for its own size. Smaller filters it down.
    pub size: Option<[u32; 2]>,
}

/// The conversion pipeline, one per target format.
pub struct VideoConverter {
    module: wgpu::ShaderModule,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
    pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
}

impl VideoConverter {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("video"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/video.wgsl").into()),
        });
        let plane = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("video_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                plane(1),
                plane(2),
                plane(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("video"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("video_chroma"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            module,
            layout,
            pipeline_layout,
            sampler,
            pipelines: Vec::new(),
        }
    }

    /// Writes `picture` into `planes`, made again when the size changed or there were none.
    pub fn upload(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        planes: &mut Option<VideoPlanes>,
        picture: &Picture,
    ) {
        let stale = planes.as_ref().is_none_or(|planes| {
            let size = planes.planes[0].size();
            (size.width, size.height) != picture.size
        });
        if stale {
            *planes = Some(self.planes(device, picture));
        }
        let Some(planes) = planes else {
            return;
        };
        let [y, u, v] = &planes.planes;
        write_plane(queue, y, &picture.y, picture.y_stride);
        write_plane(queue, u, &picture.u, picture.uv_stride);
        write_plane(queue, v, &picture.v, picture.uv_stride);
        planes.full = picture.full_range;
    }

    /// Draws `planes` into `target`; the target needs `RENDER_ATTACHMENT`.
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        planes: &VideoPlanes,
        target: VideoTarget<'_>,
    ) {
        let own = planes.planes[0].size();
        let [fill_w, fill_h] = target.size.unwrap_or([own.width, own.height]);
        if fill_w == 0 || fill_h == 0 {
            return;
        }
        let border = u32::from(target.border);
        let (width, height) = (fill_w + 2 * border, fill_h + 2 * border);
        let [x, y] = target.origin;
        let extent = target.texture.size();
        if x.saturating_add(width) > extent.width || y.saturating_add(height) > extent.height {
            return;
        }
        let params = [
            (x as f32).to_bits(),
            (y as f32).to_bits(),
            u32::from(planes.full),
            border,
            (fill_w as f32).to_bits(),
            (fill_h as f32).to_bits(),
            0,
            0,
        ];
        queue.write_buffer(&planes.params, 0, bytemuck::cast_slice(&params));
        let format = target.texture.format();
        let pipeline = self.pipeline(device, format);
        let view = target.texture.create_view(&wgpu::TextureViewDescriptor {
            base_mip_level: 0,
            mip_level_count: Some(1),
            ..Default::default()
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("video"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(x as f32, y as f32, width as f32, height as f32, 0.0, 1.0);
        pass.set_scissor_rect(x, y, width, height);
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &planes.bind, &[]);
        pass.draw(0..3, 0..1);
    }

    fn planes(&self, device: &wgpu::Device, picture: &Picture) -> VideoPlanes {
        let (width, height) = picture.size;
        let (half_w, half_h) = picture.chroma();
        let planes = [
            plane(device, width, height),
            plane(device, half_w, half_h),
            plane(device, half_w, half_h),
        ];
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("video_params"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let views = planes
            .each_ref()
            .map(|plane| plane.create_view(&Default::default()));
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("video_bg"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&views[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&views[2]),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        VideoPlanes {
            planes,
            params,
            bind,
            full: picture.full_range,
        }
    }

    fn pipeline(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        let at = match self.pipelines.iter().position(|(made, _)| *made == format) {
            Some(at) => at,
            None => {
                let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("video"),
                    layout: Some(&self.pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &self.module,
                        entry_point: Some("vs_video"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &self.module,
                        entry_point: Some(match format.is_srgb() {
                            true => "fs_video_srgb",
                            false => "fs_video",
                        }),
                        compilation_options: Default::default(),
                        targets: &[Some(format.into())],
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                });
                self.pipelines.push((format, pipeline));
                self.pipelines.len() - 1
            }
        };
        &self.pipelines[at].1
    }
}

fn plane(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("video_plane"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// Rows `stride` bytes apart, as dav1d pads them; a plane shorter than its texture is skipped.
fn write_plane(queue: &wgpu::Queue, texture: &wgpu::Texture, bytes: &[u8], stride: u32) {
    let size = texture.size();
    let needed = stride as usize * (size.height as usize - 1) + size.width as usize;
    if stride < size.width || bytes.len() < needed {
        return;
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(stride),
            rows_per_image: Some(size.height),
        },
        size,
    );
}
