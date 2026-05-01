//! Ray-march render pipeline + buffers + bind groups.
//!
//! PR-2 of #360 wires the renderer to the OmeAccel TLAS+BLAS pool.
//! Bind group 1 layout follows the issue body verbatim:
//!   - `(0)` `scene_meta` uniform — sky colours + skip-internal-sky.
//!   - `(5)` `tlas_nodes`
//!   - `(6)` `chunk_descriptors`
//!   - `(7)` `bvh_nodes_pool`
//!   - `(8)` `leaf_aabbs_pool`
//!   - `(9)` `primitives_pool`
//!   - `(10)` `tlas_uniforms`
//!   - `(11)` `gdf_cascade` (PR-4 of epic #370 — GDF cascade-0 R32F).
//!   - `(12)` `gdf_sampler` (clamp-to-edge linear).
//!   - `(13)` `gdf_uniforms` (cascade descriptor).
//!
//! Pool buffers are pre-allocated once at `BvhState::new` and never
//! reallocated; the GDF cascade texture + sampler + uniform buffer
//! are owned by `GdfState` and persistent for the renderer's lifetime
//! (the uniform buffer's contents are rewritten per frame, but the
//! buffer handle is stable). The scene bind group is built ONCE at
//! construction and stays valid for the renderer's lifetime — no
//! per-frame rebind.

use glam::Vec3;
use wgpu::util::DeviceExt;

use super::SHADER_SOURCE;
use super::bvh::BvhState;
use super::instance::{CameraUniforms, RayMarchParams, SceneMeta};
use crate::VIEWPORT_DEPTH_FORMAT;
use crate::gdf::GdfState;

/// Ray-marching pipeline + buffers + bind groups.
pub struct RayMarchRenderer {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) camera_buffer: wgpu::Buffer,
    pub(super) params_buffer: wgpu::Buffer,
    pub(super) scene_meta_buffer: wgpu::Buffer,
    pub(super) camera_bind_group: wgpu::BindGroup,
    pub(super) scene_bind_group: wgpu::BindGroup,
    pub(super) bvh_state: BvhState,
    pub(super) gdf_state: GdfState,
    /// Camera world position captured by the most recent
    /// `update_camera` call. `update_scene` reads it to centre the
    /// GDF cascade-0 voxel grid on the camera before dispatching the
    /// populate compute pass. Defaults to origin so a first-frame
    /// render with no active camera still produces a self-consistent
    /// cascade centred on `(0, 0, 0)`.
    pub(super) last_camera_pos: Vec3,
    pub params: RayMarchParams,
}

impl RayMarchRenderer {
    pub fn new(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
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

        let bvh_state = BvhState::new(device);
        let gdf_state = GdfState::new(device, bvh_state.buffers());

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
                entries: &pool_scene_bgl_entries(),
            });

        let camera_bind_group = make_camera_bg(
            device,
            &camera_bind_group_layout,
            &camera_buffer,
            &params_buffer,
        );
        let scene_bind_group = make_pool_scene_bg(
            device,
            &scene_bind_group_layout,
            &scene_meta_buffer,
            bvh_state.buffers(),
            &gdf_state,
        );

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("raymarch_pipeline_layout"),
            bind_group_layouts: &[
                Some(&camera_bind_group_layout),
                Some(&scene_bind_group_layout),
            ],
            immediate_size: 0,
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
            // LessEqual (not Less) so the sky pixel — which writes
            // `frag_depth = 1.0` explicitly — passes the depth test
            // against a depth buffer cleared to 1.0. With Less the
            // sky would fail (1.0 < 1.0 = false) and the viewport
            // would show the clear color (black) behind every SDF.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: VIEWPORT_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
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
            params_buffer,
            scene_meta_buffer,
            camera_bind_group,
            scene_bind_group,
            bvh_state,
            gdf_state,
            last_camera_pos: Vec3::ZERO,
            params: RayMarchParams::default(),
        }
    }

    /// Records the ray-march pass into `encoder` and draws the fullscreen
    /// triangle. The fragment shader writes `@builtin(frag_depth)` from the
    /// world-space hit so later mesh passes can depth-test against the SDF.
    ///
    /// When `clear_targets = true` the pass clears color to black and depth
    /// to 1.0 — appropriate when the raymarch is the FIRST pass of the
    /// frame. When `false` the targets are loaded, preserving whatever a
    /// prior pass wrote (e.g. a `SkyRenderPass` that already drew a sky
    /// background + depth=1.0); pair with `update_scene(skip_internal_sky = true)`.
    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        clear_targets: bool,
    ) {
        let color_load = if clear_targets {
            wgpu::LoadOp::Clear(wgpu::Color::BLACK)
        } else {
            wgpu::LoadOp::Load
        };
        let depth_load = if clear_targets {
            wgpu::LoadOp::Clear(1.0)
        } else {
            wgpu::LoadOp::Load
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("raymarch_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: color_load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: depth_load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        pass.set_bind_group(1, &self.scene_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

pub(super) fn make_camera_bg(
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

/// Bind-group layout entries for the pool-driven scene bind group.
/// Group 1 — `(0)` `scene_meta` uniform + `(5..=10)` pool buffers +
/// `(11..=13)` GDF cascade fetch (PR-4 of epic #370). Bindings 1..=4
/// stay absent; the legacy global-BVH bindings that used to occupy
/// those slots are gone. Pool buffers stay even though `eval_scene_bvh`
/// no longer references them — PR-8 will resurrect the traversal for
/// hybrid surface refinement, and naga happily accepts the unreferenced
/// bindings via reachability pruning during pipeline creation.
fn pool_scene_bgl_entries() -> [wgpu::BindGroupLayoutEntry; 10] {
    let storage = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only: true },
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    let uniform = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    let cascade_texture = wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable: true },
        view_dimension: wgpu::TextureViewDimension::D3,
        multisampled: false,
    };
    let cascade_sampler = wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering);
    let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty,
        count: None,
    };
    [
        entry(0, uniform),          // scene_meta
        entry(5, storage),          // tlas_nodes
        entry(6, storage),          // chunk_descriptors
        entry(7, storage),          // bvh_nodes_pool
        entry(8, storage),          // leaf_aabbs_pool
        entry(9, storage),          // primitives_pool
        entry(10, uniform),         // tlas_uniforms
        entry(11, cascade_texture), // gdf_cascade
        entry(12, cascade_sampler), // gdf_sampler
        entry(13, uniform),         // gdf_uniforms (CascadeDescriptor)
    ]
}

/// Build the pool-driven scene bind group. Pool buffers come from
/// `OmeAccel::buffers()` — pre-allocated at `BvhState::new`, never
/// reallocated. The GDF cascade texture + sampler + uniform buffer
/// come from `GdfState`, also stable for the renderer's lifetime.
/// This bind group is built ONCE at construction.
pub(super) fn make_pool_scene_bg(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    meta: &wgpu::Buffer,
    pool: &ome_bvh::AccelBuffers,
    gdf: &GdfState,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("raymarch_scene_bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: meta.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 5, resource: pool.tlas_nodes.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 6, resource: pool.chunk_descriptors.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 7, resource: pool.bvh_nodes_pool.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 8, resource: pool.leaf_aabbs_pool.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 9, resource: pool.primitives_pool.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 10, resource: pool.tlas_uniforms.as_entire_binding() },
            wgpu::BindGroupEntry {
                binding: 11,
                resource: wgpu::BindingResource::TextureView(gdf.cascade_view()),
            },
            wgpu::BindGroupEntry {
                binding: 12,
                resource: wgpu::BindingResource::Sampler(gdf.sampler()),
            },
            wgpu::BindGroupEntry {
                binding: 13,
                resource: gdf.uniforms_buffer().as_entire_binding(),
            },
        ],
    })
}
