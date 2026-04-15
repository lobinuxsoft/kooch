//! Ray-marching renderer — fullscreen fragment shader that sphere-traces
//! SDF components from the ECS.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use ome_ecs::query::Query;
use ome_ecs::{PerspectiveCamera, SdfSphere, Transform};
use wgpu::util::DeviceExt;

/// Fullscreen shader source (primitives + ray-march main), concatenated
/// at compile time.
const SHADER_SOURCE: &str = concat!(
    include_str!("../../ome_sdf/shaders/sdf_primitives.wgsl"),
    "\n",
    include_str!("../shaders/raymarch_main.wgsl"),
);

/// Matches `CameraUniforms` in `raymarch_main.wgsl`.
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

/// Matches `RayMarchParams` in the shader.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Debug)]
pub struct RayMarchParams {
    pub max_steps: u32,
    pub max_distance: f32,
    pub surface_threshold: f32,
    pub _pad: f32,
}

impl Default for RayMarchParams {
    fn default() -> Self {
        Self {
            max_steps: 128,
            max_distance: 100.0,
            surface_threshold: 0.001,
            _pad: 0.0,
        }
    }
}

/// Matches `SphereInstance` in the shader.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default)]
struct SphereInstance {
    center: [f32; 3],
    radius: f32,
}

/// Matches `SceneMeta` in the shader.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct SceneMeta {
    sphere_count: u32,
    _pad0: [u32; 3],
    sky_top: [f32; 4],
    sky_bottom: [f32; 4],
}

impl Default for SceneMeta {
    fn default() -> Self {
        Self {
            sphere_count: 0,
            _pad0: [0; 3],
            sky_top: [0.5, 0.7, 1.0, 1.0],
            sky_bottom: [0.1, 0.2, 0.4, 1.0],
        }
    }
}

/// Initial capacity for the sphere storage buffer (grows on demand).
const INITIAL_SPHERE_CAPACITY: u64 = 256;

/// Ray-marching pipeline + buffers + bind groups.
pub struct RayMarchRenderer {
    pipeline: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    params_buffer: wgpu::Buffer,
    scene_meta_buffer: wgpu::Buffer,
    sphere_buffer: wgpu::Buffer,
    sphere_capacity: u64,
    camera_bind_group_layout: wgpu::BindGroupLayout,
    scene_bind_group_layout: wgpu::BindGroupLayout,
    camera_bind_group: wgpu::BindGroup,
    scene_bind_group: wgpu::BindGroup,
    pub params: RayMarchParams,
}

impl RayMarchRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("raymarch_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("raymarch_camera_buffer"),
            contents: bytemuck::bytes_of(&CameraUniforms::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("raymarch_params_buffer"),
            contents: bytemuck::bytes_of(&RayMarchParams::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let scene_meta_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("raymarch_scene_meta_buffer"),
            contents: bytemuck::bytes_of(&SceneMeta::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let sphere_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("raymarch_sphere_buffer"),
            size: INITIAL_SPHERE_CAPACITY * std::mem::size_of::<SphereInstance>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("raymarch_camera_bgl"),
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
        let scene_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("raymarch_scene_bgl"),
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
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let camera_bind_group = Self::make_camera_bg(
            device,
            &camera_bind_group_layout,
            &camera_buffer,
            &params_buffer,
        );
        let scene_bind_group = Self::make_scene_bg(
            device,
            &scene_bind_group_layout,
            &scene_meta_buffer,
            &sphere_buffer,
        );

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("raymarch_pipeline_layout"),
            bind_group_layouts: &[&camera_bind_group_layout, &scene_bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("raymarch_pipeline"),
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
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            pipeline,
            camera_buffer,
            params_buffer,
            scene_meta_buffer,
            sphere_buffer,
            sphere_capacity: INITIAL_SPHERE_CAPACITY,
            camera_bind_group_layout,
            scene_bind_group_layout,
            camera_bind_group,
            scene_bind_group,
            params: RayMarchParams::default(),
        }
    }

    fn make_camera_bg(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        camera: &wgpu::Buffer,
        params: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raymarch_camera_bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params.as_entire_binding(),
                },
            ],
        })
    }

    fn make_scene_bg(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        meta: &wgpu::Buffer,
        spheres: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raymarch_scene_bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: meta.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: spheres.as_entire_binding(),
                },
            ],
        })
    }

    /// Uploads the active camera (from ECS) to the GPU.
    ///
    /// Picks the first `active` `PerspectiveCamera` paired with a `Transform`
    /// by highest `priority`. Returns `true` when a camera was found.
    pub fn update_camera(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &ome_core::resource::Resources,
        aspect: f32,
    ) -> bool {
        let query = Query::<(&PerspectiveCamera, &Transform)>::new(resources);
        let mut best: Option<(i32, PerspectiveCamera, Transform)> = None;
        query.for_each(|(cam, tr)| {
            if !cam.active {
                return;
            }
            let better = match &best {
                Some((p, _, _)) => cam.priority > *p,
                None => true,
            };
            if better {
                best = Some((cam.priority, *cam, *tr));
            }
        });
        drop(query);

        let Some((_, cam, tr)) = best else {
            return false;
        };

        let view = Mat4::from_rotation_translation(tr.rotation, tr.position).inverse();
        let projection = Mat4::perspective_rh(
            cam.fov.to_radians(),
            aspect.max(0.001),
            cam.near.max(0.001),
            cam.far.max(cam.near + 0.001),
        );
        let uniforms = CameraUniforms {
            view: view.to_cols_array_2d(),
            projection: projection.to_cols_array_2d(),
            inverse_view: view.inverse().to_cols_array_2d(),
            inverse_projection: projection.inverse().to_cols_array_2d(),
            position: tr.position.to_array(),
            _pad0: 0.0,
        };
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniforms));
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&self.params));

        let _ = device;
        true
    }

    /// Uploads all visible `SdfSphere + Transform` entities to the sphere
    /// storage buffer. Grows the buffer if the count exceeds capacity.
    pub fn update_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &ome_core::resource::Resources,
        sky_top: Vec4,
        sky_bottom: Vec4,
    ) {
        let query = Query::<(&SdfSphere, &Transform)>::new(resources);
        let mut spheres: Vec<SphereInstance> = Vec::new();
        query.for_each(|(sphere, tr)| {
            if !sphere.visible {
                return;
            }
            let uniform_scale = (tr.scale.x + tr.scale.y + tr.scale.z) / 3.0;
            spheres.push(SphereInstance {
                center: tr.position.to_array(),
                radius: sphere.radius * uniform_scale,
            });
        });
        drop(query);

        let needed = spheres.len().max(1) as u64;
        if needed > self.sphere_capacity {
            let new_cap = needed.next_power_of_two().max(INITIAL_SPHERE_CAPACITY);
            self.sphere_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("raymarch_sphere_buffer"),
                size: new_cap * std::mem::size_of::<SphereInstance>() as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.sphere_capacity = new_cap;
            self.scene_bind_group = Self::make_scene_bg(
                device,
                &self.scene_bind_group_layout,
                &self.scene_meta_buffer,
                &self.sphere_buffer,
            );
            let _ = &self.camera_bind_group_layout;
        }

        if !spheres.is_empty() {
            queue.write_buffer(&self.sphere_buffer, 0, bytemuck::cast_slice(&spheres));
        }

        let meta = SceneMeta {
            sphere_count: spheres.len() as u32,
            _pad0: [0; 3],
            sky_top: sky_top.to_array(),
            sky_bottom: sky_bottom.to_array(),
        };
        queue.write_buffer(&self.scene_meta_buffer, 0, bytemuck::bytes_of(&meta));
    }

    /// Records the ray-march pass into `encoder`.
    pub fn render(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("raymarch_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        pass.set_bind_group(1, &self.scene_bind_group, &[]);
        pass.draw(0..3, 0..1);
        let _ = Vec3::ZERO;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_parses() {
        let module = naga::front::wgsl::parse_str(SHADER_SOURCE)
            .expect("concatenated raymarch shader should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("concatenated raymarch shader should validate");
    }
}
