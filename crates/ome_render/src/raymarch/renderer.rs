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
use super::bind_groups::{make_camera_bg, make_pool_scene_bg, pool_scene_bgl_entries};
use super::bvh::BvhState;
use super::instance::{CameraUniforms, RayMarchParams, SceneMeta};
use crate::VIEWPORT_DEPTH_FORMAT;
use crate::gdf::{GdfScheduler, GdfState};

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
    /// Round-robin scheduler picking which cascades to populate this
    /// frame. PR-5 of epic #370. `update_scene` queries it once per
    /// frame and dispatches the returned cascades into the same
    /// encoder as the TLAS rebuild.
    pub(super) gdf_scheduler: GdfScheduler,
    /// Camera world position captured by the most recent
    /// `update_camera` call. `update_scene` reads it to centre the
    /// GDF cascades on the camera before dispatching their populate
    /// passes. Defaults to origin so a first-frame render with no
    /// active camera still produces a self-consistent cascade
    /// stack rooted at `(0, 0, 0)`.
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
            gdf_scheduler: GdfScheduler::new(),
            last_camera_pos: Vec3::ZERO,
            params: RayMarchParams::default(),
        }
    }

    /// Mutable access to the GDF cascade state. Tests use this to
    /// dispatch a populate pass before rendering when they don't drive
    /// the renderer through `update_scene` (which folds the populate
    /// into the per-frame encoder automatically).
    pub fn gdf_state_mut(&mut self) -> &mut GdfState {
        &mut self.gdf_state
    }

    /// Re-centre cascade 0 on `camera_pos` and dispatch its GDF
    /// populate compute pass for a one-shot render. Production code
    /// goes through `update_scene`; this entry point exists for
    /// tests + tools that bypass the ECS-driven update. Forces the
    /// dispatch — does NOT consult the round-robin scheduler.
    pub fn dispatch_gdf_populate(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera_pos: Vec3,
    ) {
        self.last_camera_pos = camera_pos;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("raymarch_dispatch_gdf_populate"),
        });
        self.gdf_state
            .dispatch_populate(&mut encoder, queue, camera_pos);
        queue.submit(std::iter::once(encoder.finish()));
    }

    /// Drive the round-robin scheduler one frame: ask which cascades
    /// need a populate dispatch and submit them all in a single
    /// encoder. Mirrors the GDF half of `update_scene` for tests +
    /// benchmarks that don't build an ECS world.
    pub fn dispatch_gdf_populate_scheduled(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera_pos: Vec3,
    ) {
        self.last_camera_pos = camera_pos;
        let cascades = self.gdf_scheduler.cascades_to_update(camera_pos);
        if cascades.is_empty() {
            return;
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("raymarch_dispatch_gdf_populate_scheduled"),
        });
        for cascade_idx in cascades {
            self.gdf_state.dispatch_populate_cascade(
                &mut encoder,
                queue,
                cascade_idx as usize,
                camera_pos,
            );
        }
        queue.submit(std::iter::once(encoder.finish()));
    }

    /// Mutable accessor to the GDF round-robin scheduler. Tests use
    /// it to seed dirty marks or inspect the current frame index.
    pub fn gdf_scheduler_mut(&mut self) -> &mut GdfScheduler {
        &mut self.gdf_scheduler
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

