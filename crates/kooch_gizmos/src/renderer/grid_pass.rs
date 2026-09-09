//! The infinite ground grid, drawn into the scene rather than over it.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

use crate::grid::{GridLevel, STEPS};

/// What the grid shader reads. Mirrors `GridUniforms` in `grid.wgsl`.
///
/// `#[repr(C)]` with explicit padding: a `vec3` in WGSL is 16-byte
/// aligned, so every one is followed by the scalar that shares its slot.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct GridUniforms {
    inverse_view_proj: [[f32; 4]; 4],
    view_proj: [[f32; 4]; 4],
    camera_position: [f32; 3],
    small_step: f32,
    cell_color: [f32; 3],
    steps: f32,
    counting_color: [f32; 3],
    blend: f32,
    axis_x_color: [f32; 3],
    plane_y: f32,
    axis_z_color: [f32; 3],
    fade_distance: f32,
    flags: [f32; 4],
}

// The whole reason `flags` is a vec4: a uniform's size has to agree
// with the shader's, and WGSL rounds a struct up to 16.
const _: () = assert!(size_of::<GridUniforms>() == 224);

/// How a grid should look and where it lives.
#[derive(Debug, Clone, Copy)]
pub struct GridPlane {
    /// Height of the plane. Zero for the ground.
    pub height: f32,
    /// The value the handles snap to — the finest cell it may draw.
    pub step: f32,
    pub cell: Vec3,
    pub counting: Vec3,
    /// Whether the world axes cross it.
    pub axes: bool,
}

/// Draws one horizontal plane of grid, per pixel.
pub struct GridPass {
    pipeline: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl GridPass {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grid_shader"),
            source: wgpu::ShaderSource::Wgsl(crate::GRID_SHADER.into()),
        });

        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grid_uniforms"),
            contents: bytemuck::bytes_of(&GridUniforms::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grid_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                // Both stages: the vertex half unprojects the corners,
                // the fragment half needs every colour and step.
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    // Declared so a layout that drifts from the shader's
                    // fails once, at startup, instead of every draw.
                    min_binding_size: wgpu::BufferSize::new(size_of::<GridUniforms>() as u64),
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grid_bg"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grid_pipeline_layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("grid_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                // 🔴 Tested, unlike every other gizmo. A handle draws over
                // whatever is in front of it because you have to be able
                // to grab one behind a wall; a grid doing that is a
                // lattice painted on the camera.
                //
                // `Greater` because this engine is reversed-Z.
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache,
        });

        Self {
            pipeline,
            buffer,
            bind_group,
        }
    }

    /// Draws one plane. Call once per grid, into an open pass.
    pub fn draw(
        &self,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'_>,
        view_proj: Mat4,
        camera: Vec3,
        plane: GridPlane,
    ) {
        // From the camera's distance to the plane, so the cells stay
        // about one size on screen however far out you zoom.
        let level = GridLevel::at(camera.y - plane.height, plane.step);
        queue.write_buffer(
            &self.buffer,
            0,
            bytemuck::bytes_of(&GridUniforms {
                inverse_view_proj: view_proj.inverse().to_cols_array_2d(),
                view_proj: view_proj.to_cols_array_2d(),
                camera_position: camera.to_array(),
                small_step: level.small_step,
                cell_color: plane.cell.to_array(),
                steps: STEPS,
                counting_color: plane.counting.to_array(),
                blend: level.blend,
                axis_x_color: [0.78, 0.24, 0.28],
                plane_y: plane.height,
                axis_z_color: [0.24, 0.42, 0.80],
                // Reaches as far as a hundred of the coarse cells, so
                // the fade scales with the level rather than ending at a
                // fixed metre count nobody chose.
                fade_distance: level.large_step() * 100.0,
                flags: [
                    match plane.axes {
                        true => 1.0,
                        false => 0.0,
                    },
                    0.0,
                    0.0,
                    0.0,
                ],
            }),
        );

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
