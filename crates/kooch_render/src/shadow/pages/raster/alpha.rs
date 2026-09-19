//! The depth pass's variant for transparent casters (#1224): the same vertex, and a fragment that
//! drops what the baked coverage does not reach.

use super::{PAGE_DEPTH_FORMAT, PAGE_FRONT_FACE};

const FRAGMENT: &str = include_str!("../../../../shaders/page_depth_alpha.wgsl");

/// Built beside the plain depth pipeline: `source` is that pipeline's WGSL, and the coverage's bind
/// group sits after its three.
pub(super) fn depth_alpha(
    device: &wgpu::Device,
    source: &str,
    layouts: [&wgpu::BindGroupLayout; 3],
    clipped: bool,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("page_depth_alpha"),
        source: wgpu::ShaderSource::Wgsl(
            format!(
                "{source}\n{}\n{FRAGMENT}",
                crate::shadow::shadow_alpha_shader(3)
            )
            .into(),
        ),
    });
    let alpha = crate::shadow::ShadowAlpha::bind_group_layout(device);
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("page_depth_alpha_layout"),
        bind_group_layouts: &[
            Some(layouts[0]),
            Some(layouts[1]),
            Some(layouts[2]),
            Some(&alpha),
        ],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("page_depth_alpha"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some(if clipped {
                "vs_page_clipped"
            } else {
                "vs_page"
            }),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some(if clipped {
                "fs_page_alpha_clipped"
            } else {
                "fs_page_alpha"
            }),
            targets: &[],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: PAGE_FRONT_FACE,
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: PAGE_DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Greater),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}
