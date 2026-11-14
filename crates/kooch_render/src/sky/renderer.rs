//! Sky render pipeline — procedural gradient + volumetric clouds.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use kooch_core::Guid;
use kooch_ecs::query::Query;
use kooch_ecs::sky_renderer::SkyRenderer;

use super::SHADER_SOURCE;
use super::graph::{SkyGraph, SkyView};
use crate::VIEWPORT_DEPTH_FORMAT;
use crate::material::MaterialPipeline;

/// Snapshot of an active `SkyRenderer` component used by the render pass.
/// Copy of the ECS component so the pass can drop its query borrow before
/// writing to GPU buffers.
#[derive(Debug, Clone, Copy)]
pub struct ActiveSky {
    pub top_color: [f32; 3],
    pub bottom_color: [f32; 3],
    pub sun_direction: [f32; 3],
    pub sun_color: [f32; 3],
    pub cloud_coverage: f32,
    pub cloud_density: f32,
    pub cloud_height: f32,
    pub cloud_thickness: f32,
    pub wind_direction: [f32; 3],
    pub wind_speed: f32,
    /// A `kind: sky` material, drawn instead of the fields above (#1900).
    pub material: Option<Guid>,
}

impl From<&SkyRenderer> for ActiveSky {
    fn from(s: &SkyRenderer) -> Self {
        Self {
            top_color: s.top_color.to_array(),
            bottom_color: s.bottom_color.to_array(),
            sun_direction: s.sun_direction.to_array(),
            sun_color: s.sun_color.to_array(),
            cloud_coverage: s.cloud_coverage,
            cloud_density: s.cloud_density,
            cloud_height: s.cloud_height,
            cloud_thickness: s.cloud_thickness,
            wind_direction: s.wind_direction.to_array(),
            wind_speed: s.wind_speed,
            material: s.material,
        }
    }
}

/// Where the sky draws: the colour and depth it clears, and their aspect.
pub struct SkyTarget<'a> {
    pub view: &'a wgpu::TextureView,
    pub depth: &'a wgpu::TextureView,
    pub aspect: f32,
}

/// Matches `CameraUniforms` in sky_main.wgsl — same layout as the raymarch
/// camera uniform so we can share update code.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default)]
struct CameraUniforms {
    view: [[f32; 4]; 4],
    projection: [[f32; 4]; 4],
    inverse_view: [[f32; 4]; 4],
    inverse_projection: [[f32; 4]; 4],
    position: [f32; 3],
    _pad0: f32,
}

/// Matches `SkyUniforms` in sky_main.wgsl. Tight `vec4`-packed layout
/// (96 bytes) so every field is naturally aligned in std140/std430.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct SkyUniforms {
    top_color: [f32; 4],
    bottom_color: [f32; 4],
    sun_direction: [f32; 4],
    sun_color: [f32; 4],
    // [coverage, density, height, thickness]
    cloud_params: [f32; 4],
    // [wind.x * speed, wind.y * speed, wind.z * speed, time_secs]
    wind_time: [f32; 4],
}

impl Default for SkyUniforms {
    fn default() -> Self {
        Self {
            top_color: [0.5, 0.7, 1.0, 1.0],
            bottom_color: [0.1, 0.2, 0.4, 1.0],
            sun_direction: [0.3, 0.7, -0.5, 0.0],
            sun_color: [1.0, 0.95, 0.85, 1.0],
            cloud_params: [0.0, 0.0, 80.0, 60.0],
            wind_time: [0.0, 0.0, 0.0, 0.0],
        }
    }
}

/// Sky render pass. Holds the pipeline, camera + sky uniform buffers,
/// and a single bind group. Dispatched as a fullscreen triangle.
pub struct SkyRenderPass {
    pipeline: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    sky_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    graph: SkyGraph,
}

impl SkyRenderPass {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sky_camera_buffer"),
            contents: bytemuck::bytes_of(&CameraUniforms::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let sky_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sky_uniforms_buffer"),
            contents: bytemuck::bytes_of(&SkyUniforms::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
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
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky_bg"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: sky_buffer.as_entire_binding(),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky_pipeline"),
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
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: VIEWPORT_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: pipeline_cache,
        });

        Self {
            pipeline,
            camera_buffer,
            sky_buffer,
            bind_group,
            graph: SkyGraph::new(device, format),
        }
    }

    /// Picks the highest-priority active `SkyRenderer` in the scene and
    /// returns a snapshot. Returns `None` when no active sky exists.
    pub fn active_sky(resources: &kooch_core::resource::Resources) -> Option<ActiveSky> {
        let query = Query::<&SkyRenderer>::new(resources);
        let mut best: Option<(i32, ActiveSky)> = None;
        query.for_each(|s| {
            if !s.active {
                return;
            }
            let better = match &best {
                Some((p, _)) => s.priority > *p,
                None => true,
            };
            if better {
                best = Some((s.priority, ActiveSky::from(s)));
            }
        });
        best.map(|(_, snap)| snap)
    }

    /// Uploads the active camera + sky uniforms, then records the sky render pass into `encoder`.
    /// Clears the color target and the depth buffer — this is the FIRST pass of the frame when an
    /// active SkyRenderer exists.
    #[profiling::function]
    pub fn render(
        &mut self,
        gpu: &kooch_core::gpu::GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        target: SkyTarget<'_>,
        resources: &kooch_core::resource::Resources,
        camera: &crate::ViewCamera,
        sky: ActiveSky,
    ) -> bool {
        let (device, queue) = (gpu.device(), gpu.queue());
        let SkyTarget {
            view: target,
            depth,
            aspect,
        } = target;
        let time_secs = kooch_core::time::Time::of(resources).elapsed_secs();
        if sky.material.is_none() {
            self.update_camera(queue, resources, camera, aspect);
            self.update_sky(queue, sky, time_secs);
        }
        let mut pass = Self::begin(encoder, target, depth);
        match sky.material {
            // A material that cannot draw leaves the clear, black, as a scene without a sky.
            Some(material) => {
                if let Some(materials) = resources.get::<MaterialPipeline>() {
                    let view = SkyView {
                        view: camera.view(),
                        projection: camera.projection(aspect),
                        position: camera.position(),
                        time: time_secs,
                    };
                    self.graph
                        .draw(device, queue, &mut pass, materials, material, &view);
                }
            }
            None => {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        true
    }

    /// What a scene without a sky shows: black, the same in the editor and the game (#1900).
    pub fn clear(
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) {
        drop(Self::begin(encoder, target, depth));
    }

    /// The sky's pass: colour cleared to black, depth to far.
    pub(super) fn begin<'e>(
        encoder: &'e mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) -> wgpu::RenderPass<'e> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sky_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        })
    }

    /// Uploads the matrices of the camera **the caller is rendering through**.
    fn update_camera(
        &mut self,
        queue: &wgpu::Queue,
        resources: &kooch_core::resource::Resources,
        camera: &crate::ViewCamera,
        aspect: f32,
    ) {
        let view = camera.view();
        let projection = camera.projection(aspect);
        let translation = camera.position();

        // Plumb ActiveOrigin: same pattern as raymarch + mesh. The sky is a backdrop infinity-cube
        // — universe position only matters for future per-planet atmosphere lookups (#248), so we
        // log at TRACE for now without changing the uniform layout.
        if let Some(active_origin) = resources.get::<kooch_core::coord::ActiveOrigin>() {
            let universe_pos = active_origin.coord().translated(translation.as_dvec3());
            tracing::trace!(
                target: "kooch_render::sky",
                sector = ?universe_pos.sector,
                offset = ?universe_pos.offset,
                "camera universe position"
            );
        }

        let uniforms = CameraUniforms {
            view: view.to_cols_array_2d(),
            projection: projection.to_cols_array_2d(),
            inverse_view: view.inverse().to_cols_array_2d(),
            inverse_projection: projection.inverse().to_cols_array_2d(),
            position: translation.to_array(),
            _pad0: 0.0,
        };
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    fn update_sky(&mut self, queue: &wgpu::Queue, sky: ActiveSky, time_secs: f32) {
        let wind = [
            sky.wind_direction[0] * sky.wind_speed,
            sky.wind_direction[1] * sky.wind_speed,
            sky.wind_direction[2] * sky.wind_speed,
            time_secs,
        ];
        let uniforms = SkyUniforms {
            top_color: [sky.top_color[0], sky.top_color[1], sky.top_color[2], 1.0],
            bottom_color: [
                sky.bottom_color[0],
                sky.bottom_color[1],
                sky.bottom_color[2],
                1.0,
            ],
            sun_direction: [
                sky.sun_direction[0],
                sky.sun_direction[1],
                sky.sun_direction[2],
                0.0,
            ],
            sun_color: [sky.sun_color[0], sky.sun_color[1], sky.sun_color[2], 1.0],
            cloud_params: [
                sky.cloud_coverage,
                sky.cloud_density,
                sky.cloud_height,
                sky.cloud_thickness,
            ],
            wind_time: wind,
        };
        queue.write_buffer(&self.sky_buffer, 0, bytemuck::bytes_of(&uniforms));
    }
}
