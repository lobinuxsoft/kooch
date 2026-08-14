//! Two-pass material shading (#440) — the all-fragment replacement for
//! the compute deferred on the R64 path.
//!
//! - **Pass 1** ([`Self::resolve_material_depth`]): a fullscreen fragment
//!   pass reads the R64 vbuf and writes each covered pixel's `material_id`
//!   into a `Depth16Unorm` target as `f32(id)/65535`.
//! - **Pass 2** (one draw per registered slot): a fullscreen triangle
//!   emits `slot/65535` as clip depth and `Equal`-depth-tests against the
//!   material-depth target, so only that material's pixels shade. Each
//!   pass binds the slot's own textures — no bindless array.
//!
//! Reuses the shared meshlet geometry bind group (widened to FRAGMENT
//! visibility) for group 1; builds fragment-visible layouts for the vbuf/
//! camera/screen (group 0), materials (group 2), and scene (group 3)
//! bindings, and takes the per-material texture bind group (group 4) from
//! the [`MaterialTexturePool`].

use bytemuck::bytes_of;

use crate::contact_shadow::ContactShadowUbo;
use crate::material::MaterialPipeline;
use crate::meshlet::dispatcher::MeshletCull;
use crate::meshlet::scene::MeshletScene;
use crate::meshlet::{
    MATERIAL_DEPTH_FORMAT, MATERIAL_PASS_CONTACT_DEPTH_BINDING, MATERIAL_PASS_CONTACT_UBO_BINDING,
    MATERIAL_PBR_DEFAULT_BODY, RESOLVE_MATERIAL_DEPTH_SHADER, compose_material_shader,
};

use super::{CameraUbo, DEFERRED_COLOR_FORMAT, ScreenUbo, VBUF64_FORMAT};

/// Upper bound on shading slots the per-frame screen UBO can address.
/// Matches `MaterialPipeline::DEFAULT_CAPACITY`.
const MAX_SHADING_SLOTS: u32 = 256;

/// Pass 2's pipeline, in one of its two variants.
///
/// `debug` decides whether the shader carries the debug views at all —
/// not whether it takes a branch. The production variant is compiled
/// from source that does not contain them (#743).
fn build_shading_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    debug: bool,
) -> wgpu::RenderPipeline {
    let src = compose_material_shader(MATERIAL_PBR_DEFAULT_BODY, debug);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(if debug {
            "material_pbr_default_shader_debug"
        } else {
            "material_pbr_default_shader"
        }),
        source: wgpu::ShaderSource::Wgsl(src.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("material_two_pass_shading_pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_fullscreen"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_material"),
            targets: &[Some(wgpu::ColorTargetState {
                format: DEFERRED_COLOR_FORMAT,
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
            format: MATERIAL_DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Equal),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

pub(super) struct MaterialTwoPass {
    resolve_pipeline: wgpu::RenderPipeline,
    resolve_bgl: wgpu::BindGroupLayout,
    shading_pipeline: wgpu::RenderPipeline,
    /// The same pipeline with the debug views concatenated in (#743).
    ///
    /// Built the first time somebody selects one, because a shipped game
    /// never does: it neither compiles this nor carries a byte of it in
    /// the pipeline it does run. The editor pays one compile, once, on
    /// the frame the dropdown changes.
    ///
    /// A `OnceLock` rather than an `Option` so the whole render chain
    /// stays on `&self`. Threading `&mut` up to the frame orchestrator
    /// for a field written once, if ever, would be a large change to
    /// call signatures in exchange for nothing; the steady-state cost
    /// here is one atomic load per frame.
    shading_pipeline_debug: std::sync::OnceLock<wgpu::RenderPipeline>,
    /// Kept so the debug pipeline can be built later against the exact
    /// layout the production one uses.
    shading_layout: wgpu::PipelineLayout,
    frame_bgl: wgpu::BindGroupLayout,
    materials_bgl: wgpu::BindGroupLayout,
    scene_bgl: wgpu::BindGroupLayout,
    camera_buffer: wgpu::Buffer,
    /// One `ScreenUbo` slot per material, addressed by dynamic offset so a
    /// single buffer feeds every per-material pass in one submit.
    screen_buffer: wgpu::Buffer,
    screen_stride: u64,
    /// The contact-shadow march's per-view uniform (#735).
    contact_buffer: wgpu::Buffer,
}

impl MaterialTwoPass {
    pub(super) fn new(device: &wgpu::Device, meshlet_bgl: &wgpu::BindGroupLayout) -> Self {
        use crate::material::MaterialTexturePool;

        let resolve_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("resolve_material_depth_shader"),
            source: wgpu::ShaderSource::Wgsl(RESOLVE_MATERIAL_DEPTH_SHADER.into()),
        });
        let vbuf_read = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::ReadOnly,
                format: VBUF64_FORMAT,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        };
        let storage_read = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };

        // Pass-1 group 0: vbuf(read) + visible_meshlets + instances.
        let resolve_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("resolve_material_depth_bgl"),
            entries: &[vbuf_read(0), storage_read(1), storage_read(2)],
        });

        // Pass-2 group 0: vbuf(read) + camera + dynamic-offset screen.
        let frame_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material_two_pass_frame_bgl"),
            entries: &[
                vbuf_read(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: std::num::NonZeroU64::new(
                            std::mem::size_of::<ScreenUbo>() as u64,
                        ),
                    },
                    count: None,
                },
                // Contact shadows (#735). Both here rather than in
                // Inti's group: the depth buffer is per view and that
                // group is shared across views.
                wgpu::BindGroupLayoutEntry {
                    binding: MATERIAL_PASS_CONTACT_UBO_BINDING,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(std::mem::size_of::<
                            ContactShadowUbo,
                        >()
                            as u64),
                    },
                    count: None,
                },
                // The scene depth, sampled. It is not an attachment
                // during shading — that is the material-depth target —
                // so it is free to be read, and the Hi-Z builder
                // already reads the same view.
                wgpu::BindGroupLayoutEntry {
                    binding: MATERIAL_PASS_CONTACT_DEPTH_BINDING,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        // Pass-2 group 2: materials storage.
        let materials_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material_two_pass_materials_bgl"),
            entries: &[storage_read(0)],
        });

        // Pass-2 group 3: visible_meshlets + instances.
        let scene_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material_two_pass_scene_bgl"),
            entries: &[storage_read(0), storage_read(1)],
        });

        let texture_bgl = MaterialTexturePool::bind_group_layout(device);
        // Group 5 — Inti's per-frame constants + light storage. The
        // layout is an associated function because the pipeline has to
        // be built long before any lights exist.
        let lights_bgl = kooch_lighting::GpuLights::bind_group_layout(device);

        let resolve_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("resolve_material_depth_layout"),
            bind_group_layouts: &[Some(&resolve_bgl)],
            immediate_size: 0,
        });
        let resolve_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("resolve_material_depth_pipeline"),
            layout: Some(&resolve_layout),
            vertex: wgpu::VertexState {
                module: &resolve_shader,
                entry_point: Some("vs_fullscreen"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &resolve_shader,
                entry_point: Some("fs_resolve_material_depth"),
                targets: &[],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: MATERIAL_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let shading_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("material_two_pass_shading_layout"),
            bind_group_layouts: &[
                Some(&frame_bgl),
                Some(meshlet_bgl),
                Some(&materials_bgl),
                Some(&scene_bgl),
                Some(&texture_bgl),
                Some(&lights_bgl),
            ],
            immediate_size: 0,
        });
        let shading_pipeline = build_shading_pipeline(device, &shading_layout, false);

        let align = device.limits().min_uniform_buffer_offset_alignment as u64;
        let screen_stride = align.max(std::mem::size_of::<ScreenUbo>() as u64);
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("material_two_pass_camera_ubo"),
            size: std::mem::size_of::<CameraUbo>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let screen_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("material_two_pass_screen_ubo"),
            size: screen_stride * MAX_SHADING_SLOTS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let contact_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("material_two_pass_contact_shadow_ubo"),
            size: std::mem::size_of::<ContactShadowUbo>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            resolve_pipeline,
            resolve_bgl,
            shading_pipeline,
            shading_pipeline_debug: std::sync::OnceLock::new(),
            shading_layout,
            frame_bgl,
            materials_bgl,
            scene_bgl,
            camera_buffer,
            screen_buffer,
            screen_stride,
            contact_buffer,
        }
    }

    /// Runs pass 1 (material-depth resolve) then one pass 2 draw per
    /// shading slot. Replaces `Vbuf64Deferred::shade_scene` for production
    /// (`Off`) rendering on the R64 path.
    /// The pipeline this frame draws with, compiling the debug variant
    /// the first time one is asked for.
    fn pipeline_for(&self, device: &wgpu::Device, debug_mode: u32) -> &wgpu::RenderPipeline {
        if debug_mode == 0 {
            return &self.shading_pipeline;
        }
        self.shading_pipeline_debug
            .get_or_init(|| build_shading_pipeline(device, &self.shading_layout, true))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn shade(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        vbuf_view: &wgpu::TextureView,
        material_depth_view: &wgpu::TextureView,
        depth_sample_view: &wgpu::TextureView,
        color_view: &wgpu::TextureView,
        meshlet_bg: &wgpu::BindGroup,
        cull: &MeshletCull,
        scene: &MeshletScene,
        material_pipeline: &MaterialPipeline,
        lights_bg: &wgpu::BindGroup,
        view_proj: glam::Mat4,
        contact: &ContactShadowUbo,
        screen_size: (u32, u32),
        debug_mode: u32,
    ) {
        let shading_pipeline = self.pipeline_for(device, debug_mode);

        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytes_of(&CameraUbo {
                view_proj: view_proj.to_cols_array_2d(),
            }),
        );
        queue.write_buffer(&self.contact_buffer, 0, bytes_of(contact));
        let slots = material_pipeline.shading_slots();
        debug_assert!(
            slots.end <= MAX_SHADING_SLOTS,
            "shading slot count exceeds MAX_SHADING_SLOTS",
        );
        for slot in slots.clone() {
            queue.write_buffer(
                &self.screen_buffer,
                slot as u64 * self.screen_stride,
                bytes_of(&ScreenUbo {
                    size: [screen_size.0, screen_size.1],
                    material_id: slot,
                    debug_mode,
                    // This path shades inside its own raster: one
                    // invocation per covered pixel, and no thread to
                    // remove (#825).
                    shading_rate: 1,
                    _pad: [0; 3],
                }),
            );
        }

        // Pass 1: resolve material id into the depth target.
        let resolve_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("resolve_material_depth_bg"),
            layout: &self.resolve_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(vbuf_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: cull.visible_meshlets_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: scene.instance_buffer().as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("resolve_material_depth_pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: material_depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.resolve_pipeline);
            pass.set_bind_group(0, &resolve_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        // Pass 2: per-material shading, depth-tested Equal.
        let frame_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material_two_pass_frame_bg"),
            layout: &self.frame_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(vbuf_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.screen_buffer,
                        offset: 0,
                        size: std::num::NonZeroU64::new(std::mem::size_of::<ScreenUbo>() as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: MATERIAL_PASS_CONTACT_UBO_BINDING,
                    resource: self.contact_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: MATERIAL_PASS_CONTACT_DEPTH_BINDING,
                    resource: wgpu::BindingResource::TextureView(depth_sample_view),
                },
            ],
        });
        let materials_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material_two_pass_materials_bg"),
            layout: &self.materials_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: material_pipeline.pool().buffer().as_entire_binding(),
            }],
        });
        let scene_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material_two_pass_scene_bg"),
            layout: &self.scene_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: cull.visible_meshlets_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: scene.instance_buffer().as_entire_binding(),
                },
            ],
        });

        let texture_pool = material_pipeline.texture_pool();
        for (i, slot) in slots.enumerate() {
            let refs = material_pipeline.slot_texture_refs(slot);
            let texture_bg = texture_pool.material_bind_group(device, refs[0], refs[1], refs[2]);
            let color_load = if i == 0 {
                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
            } else {
                wgpu::LoadOp::Load
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("material_two_pass_shading_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: color_load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: material_depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(shading_pipeline);
            let offset = (slot as u64 * self.screen_stride) as u32;
            pass.set_bind_group(0, &frame_bg, &[offset]);
            pass.set_bind_group(1, meshlet_bg, &[]);
            pass.set_bind_group(2, &materials_bg, &[]);
            pass.set_bind_group(3, &scene_bg, &[]);
            pass.set_bind_group(4, &texture_bg, &[]);
            pass.set_bind_group(5, lights_bg, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}
