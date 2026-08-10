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
    projection_scale_y,
};

use super::atlas::{SHADOW_DEPTH_FORMAT, ShadowAtlas};
use super::cascades::{CASCADE_COUNT, Cascade};

const SHADER_SOURCE: &str = include_str!("../../shaders/shadow_depth.wgsl");

/// A shadow gets the same geometric budget as the camera.
///
/// 🔴 This was 4×, on the reasoning that a shadow is a silhouette and
/// "nobody has ever noticed a shadow drawn from a slightly simpler
/// mesh". The owner noticed immediately, and the reasoning is wrong on
/// its own terms: a silhouette is the ONLY thing a shadow is, so
/// simplification error goes straight into the outline where nothing
/// hides it. On a lit surface the same error is a shading gradient.
///
/// Bevy 0.19 tests `norm_error * viewport_height < 1.0` for every view,
/// shadow cascades included — no relaxation term exists in their
/// selector. The budget is already measured in the cascade's own texels,
/// so a cascade covering more world already asks for less detail; that
/// relationship was doing the job this constant was invented to do, and
/// then it was applied twice.
const SHADOW_LOD_RELAXATION: f32 = 1.0;

/// No rasteriser depth bias.
///
/// 🔴 The bias moved into the shading pass, in world space, where Bevy
/// 0.19 keeps both of theirs (`shadow_depth_bias` along the direction to
/// the light, `shadow_normal_bias` along the surface normal). Bevy sets
/// no `DepthBiasState` on its shadow pipeline at all.
///
/// Running both is how a shadow ends up detached from its object *and*
/// still showing acne elsewhere: each bias is tuned against artifacts
/// the other is already half-hiding, so neither ends up at a value that
/// is right on its own.
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
        // Clamp depth rather than clip it, so an occluder nearer the
        // light than the cascade's near plane is still recorded at the
        // near plane instead of vanishing. That is what lets the depth
        // range hug the slice — see `build_cascades`. Optional: the
        // fallback is a near-plane margin, which costs precision rather
        // than correctness.
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
            // Cascades, then one slot per spot light's shadow (#777),
            // on the same index scheme as the array's layers.
            size: cascade_stride * (CASCADE_COUNT + kooch_lighting::MAX_SPOT_SHADOWS) as u64,
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

    /// How far past its own nearest point a cascade's near plane has to
    /// sit, as a fraction of the cascade's width.
    ///
    /// Zero when the pipeline clamps depth: an occluder in front of the
    /// near plane is recorded at it rather than clipped away, so there
    /// is nothing to make room for.
    pub fn near_extension_scale(&self) -> f32 {
        if self.unclipped_depth { 0.0 } else { 1.0 }
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
                        // Reversed-Z: 0 is the far plane, so an empty
                        // cascade reads as "nothing between here and the
                        // light" rather than as "everything is shadowed".
                        // Clearing to 1 would put the whole scene in
                        // shadow the first frame a cascade draws nothing.
                        //
                        // Every layer clears, where the atlas had only
                        // cascade 0 clear (it owned the whole texture)
                        // and the rest load. A layer is its own
                        // attachment and loading here would keep last
                        // frame's depths.
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
            // The atlas needed both, because a viewport clips geometry
            // but does not stop a clear or a depth bias from reaching
            // the rest of the attachment, and a cascade bleeding into
            // its neighbour's quadrant read as a shadow from the wrong
            // distance. Layers cannot touch each other.
            pass.set_pipeline(&self.pipeline);
            let offset = (i as u64 * self.cascade_stride) as u32;
            pass.set_bind_group(0, &cascade_bg, &[offset]);
            pass.set_bind_group(1, meshlet_bg, &[]);
            pass.set_bind_group(2, &visible_bg, &[]);
            pass.set_bind_group(3, &instances_bg, &[]);
            pass.draw_indirect(cull.indirect_args_buffer(), 0);
        }
    }

    /// Culls and draws every shadow-casting spot light into its own
    /// layer (#777).
    ///
    /// Separate from [`Self::render`] rather than folded into it: a
    /// cascade is orthographic and a spot is not, and the two differ in
    /// the one place that matters to the cull — how a simplification
    /// error in metres becomes an error in pixels.
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
            // The light's own position, and here it is the real eye of a
            // real perspective rather than the stand-in an orthographic
            // cascade needs. The LOD selector is left on its distance
            // form for the same reason: a spot has a viewpoint, so a
            // simplification error projects to pixels the ordinary way.
            let view_proj = glam::Mat4::from_cols_array_2d(&spot.record.view_proj);
            // 🔴 The LOD selector, which `CullParams::new` leaves at a
            // factor of ZERO — and a factor of zero does not mean "no
            // LOD", it means every meshlet's projected error is 0 px, so
            // the selector keeps only roots. `projection_scale_y`'s own
            // doc has the symptom: "a sphere collapses to a blob and a
            // cube to a spike". That is precisely what the first smoke
            // saw in the sphere's shadow.
            //
            // Perspective, so `with_lod` rather than the cascades'
            // orthographic form: a spot has a viewpoint and a
            // simplification error really does shrink with distance.
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
}

#[cfg(test)]
mod tests;
