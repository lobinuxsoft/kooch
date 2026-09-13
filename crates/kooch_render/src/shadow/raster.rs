//! Rendering the cascades: cull from the light, then draw depth.

use bytemuck::{Pod, Zeroable};

use crate::meshlet::{
    CullParams, GpuGlobalMeshPool, MeshletCullPipelines, MeshletScene, SceneCullParams,
    projection_scale_y,
};

use super::atlas::{SHADOW_DEPTH_FORMAT, ShadowAtlas};
use super::cascades::{CASCADE_COUNT, Cascade};
use super::cube::PointShadowCubes;
use super::point::{CUBE_FACES, PointShadowDraw};

/// Where the point lights' face matrices start in the shared uniform,
/// behind the cascades and the spots.
const POINT_UBO_BASE: usize = CASCADE_COUNT + kooch_lighting::MAX_SPOT_SHADOWS;

const SHADER_SOURCE: &str = include_str!("../../shaders/shadow_depth.wgsl");

/// A shadow gets the same geometric budget as the camera.
const SHADOW_LOD_RELAXATION: f32 = 1.0;

/// No rasteriser depth bias.
const DEPTH_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState {
    constant: 0,
    slope_scale: 0.0,
    clamp: 0.0,
};

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
struct CascadeUbo {
    view_proj: [[f32; 4]; 4],
}

/// The depth-only pipeline and the per-cascade uniforms.
pub struct ShadowRasterizer {
    pipeline: wgpu::RenderPipeline,
    /// Whether the pipeline clamps depth instead of clipping it. When it
    /// does, a cascade needs no near-plane margin at all.
    unclipped_depth: bool,
    cascade_bgl: wgpu::BindGroupLayout,
    visible_bgl: wgpu::BindGroupLayout,
    instances_bgl: wgpu::BindGroupLayout,
    /// One matrix per cascade, in one buffer addressed by dynamic
    /// offset. Four small buffers would be four bind groups; this is one
    /// bind group and an offset per pass.
    cascade_buffer: wgpu::Buffer,
    cascade_stride: u64,
}

impl ShadowRasterizer {
    pub fn new(device: &wgpu::Device, meshlet_bgl: &wgpu::BindGroupLayout) -> Self {
        // Clamp depth rather than clip it, so an occluder nearer the light than the cascade's near
        // plane is still recorded at the near plane instead of vanishing. That is what lets the
        // depth range hug the slice — see `build_cascades`.
        let unclipped_depth = device
            .features()
            .contains(wgpu::Features::DEPTH_CLIP_CONTROL);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shadow_depth_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let cascade_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow_cascade_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: std::num::NonZeroU64::new(64),
                },
                count: None,
            }],
        });
        let storage_vertex = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let visible_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow_visible_bgl"),
            entries: &[storage_vertex(0)],
        });
        let instances_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow_instances_bgl"),
            entries: &[storage_vertex(0)],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow_depth_pipeline_layout"),
            bind_group_layouts: &[
                Some(&cascade_bgl),
                Some(meshlet_bgl),
                Some(&visible_bgl),
                Some(&instances_bgl),
            ],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow_depth_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_shadow"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            // No fragment stage at all. Depth goes through fixed-function hardware; there is no
            // invocation to skip and nothing to disable early-Z. See shadow_depth.wgsl for what
            // that costs (alpha-cut geometry does not cut).
            fragment: None,
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // Back-face culling, the same as the main pass.
                cull_mode: Some(wgpu::Face::Back),
                unclipped_depth,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                // Reversed-Z, like every other depth test in the engine.
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: wgpu::StencilState::default(),
                // None: the bias lives in the shading pass, in world
                // space, the way Bevy 0.19 does it. See `DEPTH_BIAS`.
                bias: DEPTH_BIAS,
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let align = device.limits().min_uniform_buffer_offset_alignment as u64;
        let cascade_stride = align.max(std::mem::size_of::<CascadeUbo>() as u64);
        let cascade_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow_cascade_ubo"),
            // Cascades, then one slot per spot light's shadow (#777), on the same index scheme as
            // the array's layers, then six per point light (#778).
            size: cascade_stride * POINT_UBO_BASE as u64
                + cascade_stride * (kooch_lighting::MAX_POINT_SHADOWS * CUBE_FACES) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            unclipped_depth,
            cascade_bgl,
            visible_bgl,
            instances_bgl,
            cascade_buffer,
            cascade_stride,
        }
    }

    /// How far past its own nearest point a cascade's near plane has to sit, as a fraction of the
    /// cascade's width.
    pub fn near_extension_scale(&self) -> f32 {
        if self.unclipped_depth { 0.0 } else { 1.0 }
    }

    /// Culls and draws every cascade into the atlas.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        atlas: &ShadowAtlas,
        cascades: &[Cascade; CASCADE_COUNT],
        cull_pipelines: &MeshletCullPipelines,
        pool: &GpuGlobalMeshPool,
        scene: &MeshletScene,
        meshlet_bg: &wgpu::BindGroup,
        instance_count: u32,
        max_meshlets_per_mesh: u32,
        lod_target: f32,
    ) {
        for (i, cascade) in cascades.iter().enumerate() {
            queue.write_buffer(
                &self.cascade_buffer,
                i as u64 * self.cascade_stride,
                bytemuck::bytes_of(&CascadeUbo {
                    view_proj: cascade.view_proj.to_cols_array_2d(),
                }),
            );
        }

        let scene_params = SceneCullParams::new(instance_count, max_meshlets_per_mesh);
        let cascade_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow_cascade_bg"),
            layout: &self.cascade_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &self.cascade_buffer,
                    offset: 0,
                    size: std::num::NonZeroU64::new(std::mem::size_of::<CascadeUbo>() as u64),
                }),
            }],
        });
        let instances_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow_instances_bg"),
            layout: &self.instances_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: scene.instance_buffer().as_entire_binding(),
            }],
        });

        // Every cull first, then every draw. The culls write buffers the draws read, so
        // interleaving them would put a barrier between each pair and serialise four cascades that
        // have no reason to wait for each other.
        for (i, cascade) in cascades.iter().enumerate() {
            let cull = atlas.cull(i);
            // 🔴 The light's eye, not the origin.
            let params =
                CullParams::new(cascade.view_proj, cascade.light_eye, max_meshlets_per_mesh)
                    // The cascade's world height, which under an orthographic projection is the
                    // entire relationship between a simplification error and how much of the shadow
                    // map it covers.
                    .with_orthographic_lod(
                        atlas.cascade_size() as f32 * cascade.texel_world_size,
                        atlas.cascade_size() as f32,
                        (lod_target * SHADOW_LOD_RELAXATION).max(0.01),
                    );
            cull.dispatch_scene_pool_atomic(
                cull_pipelines,
                device,
                queue,
                encoder,
                pool,
                scene,
                &params,
                &scene_params,
            );
        }

        for i in 0..CASCADE_COUNT {
            let cull = atlas.cull(i);
            let visible_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("shadow_visible_bg"),
                layout: &self.visible_bgl,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: cull.visible_meshlets_buffer().as_entire_binding(),
                }],
            });

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow_cascade_pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: atlas.layer_view(i),
                    depth_ops: Some(wgpu::Operations {
                        // Reversed-Z: 0 is the far plane, so an empty cascade reads as "nothing
                        // between here and the light" rather than as "everything is shadowed".
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // No viewport and no scissor: the layer IS the cascade.
            pass.set_pipeline(&self.pipeline);
            let offset = (i as u64 * self.cascade_stride) as u32;
            pass.set_bind_group(0, &cascade_bg, &[offset]);
            pass.set_bind_group(1, meshlet_bg, &[]);
            pass.set_bind_group(2, &visible_bg, &[]);
            pass.set_bind_group(3, &instances_bg, &[]);
            pass.draw_indirect(cull.indirect_args_buffer(), 0);
        }
    }

    /// Culls and draws every shadow-casting spot light into its own layer (#777).
    #[allow(clippy::too_many_arguments)]
    pub fn render_spots(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        atlas: &ShadowAtlas,
        spots: &[super::spot::SpotShadowDraw],
        cull_pipelines: &MeshletCullPipelines,
        pool: &GpuGlobalMeshPool,
        scene: &MeshletScene,
        meshlet_bg: &wgpu::BindGroup,
        instance_count: u32,
        max_meshlets_per_mesh: u32,
        lod_target: f32,
    ) {
        if spots.is_empty() {
            return;
        }
        for (slot, spot) in spots.iter().enumerate() {
            queue.write_buffer(
                &self.cascade_buffer,
                (CASCADE_COUNT + slot) as u64 * self.cascade_stride,
                bytemuck::bytes_of(&CascadeUbo {
                    view_proj: spot.record.view_proj,
                }),
            );
        }

        let scene_params = SceneCullParams::new(instance_count, max_meshlets_per_mesh);
        let cascade_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow_spot_bg"),
            layout: &self.cascade_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &self.cascade_buffer,
                    offset: 0,
                    size: std::num::NonZeroU64::new(std::mem::size_of::<CascadeUbo>() as u64),
                }),
            }],
        });
        let instances_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow_spot_instances_bg"),
            layout: &self.instances_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: scene.instance_buffer().as_entire_binding(),
            }],
        });

        // Every cull, then every draw — the same reason the cascades do
        // it: a draw reads the survivor list the next cull writes, so
        // interleaving puts a barrier between each pair.
        for (slot, spot) in spots.iter().enumerate() {
            let cull = atlas.spot_cull(slot);
            // The light's own position, and here it is the real eye of a real perspective rather
            // than the stand-in an orthographic cascade needs.
            let view_proj = glam::Mat4::from_cols_array_2d(&spot.record.view_proj);
            // 🔴 The LOD selector, which `CullParams::new` leaves at a factor of ZERO — and a factor
            // of zero does not mean "no LOD", it means every meshlet's projected error is 0 px, so
            // the selector keeps only roots.
            let params = CullParams::new(view_proj, spot.eye, max_meshlets_per_mesh).with_lod(
                atlas.cascade_size() as f32,
                projection_scale_y(view_proj),
                (lod_target * SHADOW_LOD_RELAXATION).max(0.01),
            );
            cull.dispatch_scene_pool_atomic(
                cull_pipelines,
                device,
                queue,
                encoder,
                pool,
                scene,
                &params,
                &scene_params,
            );
        }

        for (slot, _) in spots.iter().enumerate() {
            let cull = atlas.spot_cull(slot);
            let visible_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("shadow_spot_visible_bg"),
                layout: &self.visible_bgl,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: cull.visible_meshlets_buffer().as_entire_binding(),
                }],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow_spot_pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: atlas.spot_layer_view(slot),
                    depth_ops: Some(wgpu::Operations {
                        // Reversed-Z: 0 is far, so an empty map reads as
                        // "nothing between here and the light".
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            let offset = ((CASCADE_COUNT + slot) as u64 * self.cascade_stride) as u32;
            pass.set_bind_group(0, &cascade_bg, &[offset]);
            pass.set_bind_group(1, meshlet_bg, &[]);
            pass.set_bind_group(2, &visible_bg, &[]);
            pass.set_bind_group(3, &instances_bg, &[]);
            pass.draw_indirect(cull.indirect_args_buffer(), 0);
        }
    }

    /// Renders every casting point light's six faces (#778).
    #[allow(clippy::too_many_arguments)]
    pub fn render_points(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        cubes: &PointShadowCubes,
        points: &[(usize, PointShadowDraw)],
        cull_pipelines: &MeshletCullPipelines,
        pool: &GpuGlobalMeshPool,
        scene: &MeshletScene,
        meshlet_bg: &wgpu::BindGroup,
        instance_count: u32,
        max_meshlets_per_mesh: u32,
        lod_target: f32,
    ) {
        if points.is_empty() {
            return;
        }
        for (slot, light) in points.iter() {
            for (face, view_proj) in light.faces.iter().enumerate() {
                queue.write_buffer(
                    &self.cascade_buffer,
                    self.point_ubo_offset(*slot, face),
                    bytemuck::bytes_of(&CascadeUbo {
                        view_proj: view_proj.to_cols_array_2d(),
                    }),
                );
            }
        }

        let scene_params = SceneCullParams::new(instance_count, max_meshlets_per_mesh);
        let cascade_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow_point_bg"),
            layout: &self.cascade_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &self.cascade_buffer,
                    offset: 0,
                    size: std::num::NonZeroU64::new(std::mem::size_of::<CascadeUbo>() as u64),
                }),
            }],
        });
        let instances_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow_point_instances_bg"),
            layout: &self.instances_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: scene.instance_buffer().as_entire_binding(),
            }],
        });

        for (slot, light) in points.iter() {
            for (face, view_proj) in light.faces.iter().enumerate() {
                // Perspective with a real eye, so the LOD selector takes its distance form —
                // `with_lod`, not the cascades' orthographic one.
                let params = CullParams::new(*view_proj, light.eye, max_meshlets_per_mesh)
                    .with_lod(
                        cubes.size() as f32,
                        projection_scale_y(*view_proj),
                        (lod_target * SHADOW_LOD_RELAXATION).max(0.01),
                    );
                cubes.cull(face).dispatch_scene_pool_atomic(
                    cull_pipelines,
                    device,
                    queue,
                    encoder,
                    pool,
                    scene,
                    &params,
                    &scene_params,
                );
            }

            for face in 0..CUBE_FACES {
                let cull = cubes.cull(face);
                let visible_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("shadow_point_visible_bg"),
                    layout: &self.visible_bgl,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: cull.visible_meshlets_buffer().as_entire_binding(),
                    }],
                });
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("shadow_point_pass"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: cubes.face_view(*slot, face),
                        depth_ops: Some(wgpu::Operations {
                            // Reversed-Z: 0 is far, so an empty face
                            // reads as "nothing between here and the
                            // light" rather than as a wall.
                            load: wgpu::LoadOp::Clear(0.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &cascade_bg, &[self.point_ubo_offset(*slot, face) as u32]);
                pass.set_bind_group(1, meshlet_bg, &[]);
                pass.set_bind_group(2, &visible_bg, &[]);
                pass.set_bind_group(3, &instances_bg, &[]);
                pass.draw_indirect(cull.indirect_args_buffer(), 0);
            }
        }
    }

    /// Byte offset of one face's matrix in the shared uniform.
    fn point_ubo_offset(&self, slot: usize, face: usize) -> u64 {
        (POINT_UBO_BASE + slot * CUBE_FACES + face) as u64 * self.cascade_stride
    }
}

#[cfg(test)]
mod tests;
