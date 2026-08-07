//! Rendering the cascades: cull from the light, then draw depth.
//!
//! One `begin_render_pass` per cascade rather than one pass with four
//! viewports. A viewport can be changed inside a pass, but the depth
//! **clear** cannot: `LoadOp::Clear` applies to the whole attachment, so
//! a single pass would clear the atlas four times and leave only the
//! last cascade. Clearing once and switching viewport works and hides a
//! trap — the second cascade would silently depth-test against the
//! first's leftovers wherever their quadrants touch. Four passes cost
//! four begin/end and are obviously correct.

use bytemuck::{Pod, Zeroable};

use crate::meshlet::{
    CullParams, GpuGlobalMeshPool, MeshletCullPipelines, MeshletScene, SceneCullParams,
};

use super::atlas::{SHADOW_DEPTH_FORMAT, ShadowAtlas};
use super::cascades::{CASCADE_COUNT, Cascade};

const SHADER_SOURCE: &str = include_str!("../../shaders/shadow_depth.wgsl");

/// How much coarser a shadow's level of detail is than the camera's.
///
/// A shadow is a silhouette projected onto other geometry: it loses the
/// detail that a surface facing the camera keeps, and nobody has ever
/// noticed a shadow drawn from a slightly simpler mesh. Four times the
/// screen-space error budget roughly halves the triangles per cascade,
/// and there are four cascades.
const SHADOW_LOD_RELAXATION: f32 = 4.0;

/// Constant and slope-scaled depth bias, in hardware.
///
/// 🔴 **Negative because depth is reversed.** Pushing a recorded
/// occluder *away* from the light means a SMALLER value under
/// reversed-Z. Flip the sign and the bias pulls occluders toward the
/// light instead, which presents as shadow acne — so it reads as "the
/// shadow map needs more resolution" rather than as a sign error, and
/// people spend a day on the wrong thing.
const DEPTH_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState {
    constant: -2,
    slope_scale: -2.0,
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
            // No fragment stage at all. Depth goes through fixed-function
            // hardware; there is no invocation to skip and nothing to
            // disable early-Z. See shadow_depth.wgsl for what that costs
            // (alpha-cut geometry does not cut).
            fragment: None,
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // Back-face culling, the same as the main pass.
                //
                // Front-face culling is the classic shadow trick — it
                // pushes recorded depth behind the lit surface and hides
                // acne — and it trades acne for peter-panning and breaks
                // outright on single-sided geometry, which a plane and a
                // leaf both are. Modern practice is back faces plus a
                // slope-scaled bias, and the bias is where the fix
                // belongs.
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                // Reversed-Z, like every other depth test in the engine.
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: wgpu::StencilState::default(),
                // Constant and slope-scaled bias in hardware. Cheaper
                // and more accurate than offsetting in the shader, which
                // would also have to run a fragment stage to do it.
                //
                // Negative because depth is reversed: pushing a recorded
                // occluder *away* from the light means a smaller value
                // here, and the sign flipping with the convention is a
                // bug that presents as shadow acne rather than as a sign
                // error.
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
            size: cascade_stride * CASCADE_COUNT as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            cascade_bgl,
            visible_bgl,
            instances_bgl,
            cascade_buffer,
            cascade_stride,
        }
    }

    /// Culls and draws every cascade into the atlas.
    ///
    /// `lod_target` is the camera's, relaxed by
    /// [`SHADOW_LOD_RELAXATION`]: a shadow is a silhouette, and it loses
    /// the detail a surface facing the camera keeps.
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

        // Every cull first, then every draw. The culls write buffers the
        // draws read, so interleaving them would put a barrier between
        // each pair and serialise four cascades that have no reason to
        // wait for each other.
        for (i, cascade) in cascades.iter().enumerate() {
            let cull = atlas.cull(i);
            // 🔴 The light's eye, not the origin.
            //
            // The projection is orthographic and has no eye, which is
            // why this used to pass `Vec3::ZERO` — and the cull pass
            // measures from a point twice regardless. Its backface cone
            // test then rejected every meshlet whose normals face away
            // from the world origin rather than away from the sun, so
            // those meshlets wrote no depth and the shadow came out with
            // pieces missing. It reads as "some meshlets cannot cast",
            // which is exactly what it was.
            let params =
                CullParams::new(cascade.view_proj, cascade.light_eye, max_meshlets_per_mesh)
                    // The cascade's world height, which under an
                    // orthographic projection is the entire relationship
                    // between a simplification error and how much of the
                    // shadow map it covers.
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
            let region = atlas.region(i);
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
                    view: atlas.view(),
                    depth_ops: Some(wgpu::Operations {
                        // Reversed-Z: 0 is the far plane, so an empty
                        // cascade reads as "nothing between here and the
                        // light" rather than as "everything is shadowed".
                        // Clearing to 1 would put the whole scene in
                        // shadow the first frame a cascade draws nothing.
                        load: if i == 0 {
                            wgpu::LoadOp::Clear(0.0)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // The scissor as well as the viewport: a viewport clips
            // geometry but does not stop a clear or a bias from touching
            // the rest of the attachment, and a cascade writing into its
            // neighbour's quadrant reads as a shadow from the wrong
            // distance.
            pass.set_viewport(
                region.x as f32,
                region.y as f32,
                region.size as f32,
                region.size as f32,
                0.0,
                1.0,
            );
            pass.set_scissor_rect(region.x, region.y, region.size, region.size);
            pass.set_pipeline(&self.pipeline);
            let offset = (i as u64 * self.cascade_stride) as u32;
            pass.set_bind_group(0, &cascade_bg, &[offset]);
            pass.set_bind_group(1, meshlet_bg, &[]);
            pass.set_bind_group(2, &visible_bg, &[]);
            pass.set_bind_group(3, &instances_bg, &[]);
            pass.draw_indirect(cull.indirect_args_buffer(), 0);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shadow_shader_parses_and_validates() {
        let module = naga::front::wgsl::parse_str(super::SHADER_SOURCE)
            .expect("shadow_depth.wgsl should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("shadow_depth.wgsl should validate");
    }

    /// Reversed-Z runs through the whole engine, and the bias is the
    /// easiest place to get it backwards. A positive bias here pulls
    /// occluders toward the light and produces acne — which reads as a
    /// resolution problem and costs a day.
    #[test]
    fn the_depth_bias_is_signed_for_reversed_z() {
        assert!(
            super::DEPTH_BIAS.constant < 0,
            "constant bias must be negative under reversed-Z, is {}",
            super::DEPTH_BIAS.constant,
        );
        assert!(
            super::DEPTH_BIAS.slope_scale < 0.0,
            "slope-scaled bias must be negative under reversed-Z, is {}",
            super::DEPTH_BIAS.slope_scale,
        );
    }

    /// A shadow is a silhouette and loses detail a lit surface keeps, so
    /// its geometry budget is looser than the camera's — never tighter,
    /// which would spend triangles nobody can see.
    #[test]
    fn shadows_use_a_looser_lod_budget_than_the_camera() {
        assert!(super::SHADOW_LOD_RELAXATION > 1.0);
    }
}
