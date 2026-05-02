//! Composes the meshlet stage's `Rgba8Unorm` color texture onto an
//! arbitrary RENDER_ATTACHMENT view.
//!
//! Phase 1.E.3b helper. The stage's deferred shader writes through a
//! storage-texture binding which forces the output format
//! ([`DEFERRED_COLOR_FORMAT`]). When the destination — the swapchain
//! surface or the editor's `ViewportTarget` — uses a different format
//! (typically `Bgra8Unorm`), wgpu refuses both `copy_texture_to_texture`
//! (cross-format) and direct binding (storage format mismatch). The
//! cheapest portable answer is a triangle-cover blit pass.
//!
//! One [`MeshletBlit`] per destination format. Construct it once per
//! viewport / surface and reuse across frames; [`MeshletBlit::blit`]
//! records a single render pass that consumes the stage's color view
//! and writes the destination view.

use super::deferred::DEFERRED_COLOR_FORMAT;

const SHADER_SOURCE: &str = include_str!("../../shaders/meshlet_blit.wgsl");

/// Owns the blit render pipeline + sampler. Build one per destination
/// `wgpu::TextureFormat`.
pub struct MeshletBlit {
    pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    target_format: wgpu::TextureFormat,
}

impl MeshletBlit {
    /// Builds a blit pipeline that writes to a render attachment of
    /// `target_format`. The source view is sampled as
    /// [`DEFERRED_COLOR_FORMAT`] (`Rgba8Unorm`); wgpu's swizzle handles
    /// the channel reorder when targeting `Bgra8Unorm`.
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("meshlet_blit_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshlet_blit_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meshlet_blit_pipeline_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("meshlet_blit_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_blit"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_blit"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("meshlet_blit_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        Self {
            pipeline,
            bgl,
            sampler,
            target_format,
        }
    }

    pub fn target_format(&self) -> wgpu::TextureFormat {
        self.target_format
    }

    /// Records the blit render pass. The source view must reference a
    /// texture in [`DEFERRED_COLOR_FORMAT`]; the destination view must
    /// match this blit's `target_format`.
    pub fn blit(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        destination: &wgpu::TextureView,
    ) {
        debug_assert_eq!(
            DEFERRED_COLOR_FORMAT,
            wgpu::TextureFormat::Rgba8Unorm,
            "blit source binding type assumes Rgba8Unorm"
        );

        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_blit_bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("meshlet_blit_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: destination,
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bg, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blit_shader_parses_and_validates() {
        let module = naga::front::wgsl::parse_str(SHADER_SOURCE)
            .expect("meshlet_blit.wgsl should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("meshlet_blit.wgsl should validate");
    }
}
