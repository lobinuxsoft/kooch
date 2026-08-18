//! Mip chains for material textures.
//!
//! The engine uploaded every texture with `mip_level_count: 1` from the
//! first PR that had textures at all, with a comment saying the chain
//! would land "when PBR materials need them". They needed them: a
//! 1024-pixel grid on a floor in perspective samples a different texel
//! every frame and boils, and `render_scale` below 100 makes it worse by
//! exactly the ratio.
//!
//! # It is also the thing `MipBias` was blocked on
//!
//! A negative LOD bias is how a temporal upscaler gets back the high
//! frequencies a reduced render resolution throws away (#481). A bias
//! applied to a chain of one level selects level zero, which is what it
//! already selected — so the setting could not have worked, and would
//! have been debugged as an upscaler problem.
//!
//! # Why a pipeline cache rather than one pipeline
//!
//! The format has to match the texture being written: an `Rgba8UnormSrgb`
//! attachment and an `Rgba8Unorm` one need different pipelines, and the
//! whole point of doing this on the GPU is that the format carries the
//! transfer function. Two entries in practice, keyed properly so a third
//! costs nothing.

use std::collections::HashMap;

const SHADER_SOURCE: &str = include_str!("../../shaders/mip_blit.wgsl");

#[cfg(test)]
mod tests;

/// How many levels a texture of `size` can hold, including level zero.
///
/// The chain stops at 1x1. `size` of zero cannot happen — wgpu rejects
/// a zero-sized texture — but it is floored anyway rather than trusted,
/// since the count feeds a texture descriptor.
pub fn level_count(width: u32, height: u32) -> u32 {
    let largest = width.max(height).max(1);
    32 - largest.leading_zeros()
}

/// Builds mip chains by repeatedly halving with the hardware filter.
///
/// Owned by whoever owns the textures — [`MaterialTexturePool`](crate::material::MaterialTexturePool)
/// — rather than constructed per upload: it caches a render pipeline per
/// format, and building one of those per texture would make importing a
/// folder of 78 textures pay for 78 pipeline compilations.
pub struct Mipmapper {
    module: wgpu::ShaderModule,
    bgl: wgpu::BindGroupLayout,
    layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
}

impl Mipmapper {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mip_blit_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mip_blit_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        // Filterable, and that is the entire technique:
                        // one bilinear tap at the shared corner of four
                        // texels is their average.
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mip_blit_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        // ClampToEdge, not Repeat: the source's edge texels have no
        // neighbour on the far side, and wrapping would fold the
        // opposite edge of the image into the border of every level.
        // The material sampler tiles; this one reads.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("mip_blit_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            module,
            bgl,
            layout,
            sampler,
            pipelines: HashMap::new(),
        }
    }

    fn pipeline(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        self.pipelines.entry(format).or_insert_with(|| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("mip_blit_pipeline"),
                layout: Some(&self.layout),
                vertex: wgpu::VertexState {
                    module: &self.module,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &self.module,
                    entry_point: Some("fs_main"),
                    targets: &[Some(format.into())],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        })
    }

    /// Fills levels 1.. of `texture` from level 0.
    ///
    /// Submits its own command buffer: the caller uploaded level zero
    /// with `queue.write_texture`, which is ordered against the queue
    /// rather than against an encoder, and a chain built in an encoder
    /// the caller submits later would read a level that is not there
    /// yet. One texture is one submission; this runs at import, not per
    /// frame.
    pub fn generate(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
    ) {
        let levels = texture.mip_level_count();
        if levels < 2 {
            return;
        }
        let format = texture.format();
        // Cloned out of the cache because the render pass borrows
        // `self.bgl` and `self.sampler` while the pipeline is borrowed
        // from `self.pipelines`, and one `&mut self` cannot lend both.
        let pipeline = self.pipeline(device, format).clone();

        let views: Vec<wgpu::TextureView> = (0..levels)
            .map(|level| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("mip_level"),
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("mip_chain_encoder"),
        });
        for level in 1..levels as usize {
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("mip_blit_bg"),
                layout: &self.bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&views[level - 1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mip_blit_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &views[level],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Every texel of the level is written.
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit(Some(encoder.finish()));
    }
}
