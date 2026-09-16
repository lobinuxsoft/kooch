//! The Shader Graph's preview (#1159): the open graph's shader, on a primitive, in a target of its
//! own.
//!
//! 🔴 It runs the **same** `surface` function the scene runs, through `compose_preview_shader` —
//! not a second implementation of it. What differs is the frame around it: one primitive
//! rasterised, one key light, and none of Inti, the shadow pages or the visibility buffer.

use std::hash::{Hash, Hasher};

use glam::{Mat4, Vec3};
use kooch_core::gpu::GpuContext;
use kooch_render::material::{
    MAX_PARAM_SCALARS, MaterialParams, MaterialPool, MaterialTexturePool, ParamKind, ShaderParam,
    TextureRef,
};
use kooch_render::mesh::Primitive;
use kooch_render::meshlet::compose_preview_shader;

use crate::viewport::target::ViewportTarget;

mod mesh;

use mesh::{PreviewMesh, PreviewVertex, default_primitive, upload};

/// What the Shader Graph panel asks of the preview for the next frame.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PreviewRequest {
    /// Which of `Primitive::CANONICAL` to show.
    pub primitive: usize,
    /// The target's side in pixels, once the column has settled — `None` while it is dragged.
    pub size: Option<u32>,
}

/// What the preview starts at, before the column has reported a size. After that the target
/// follows the column, re-created once a drag settles rather than on every frame of it.
const SIZE: (u32, u32) = (320, 320);

/// One turn every this many seconds, so a shader that depends on the view angle shows it.
const TURN_SECONDS: f32 = 12.0;

/// The uniforms the preview frame declares, one buffer each.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUbo {
    view_proj: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ScreenUbo {
    material_id: u32,
    mip_bias_scale: f32,
    time: f32,
    _pad: u32,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct IntiUbo {
    camera_position: [f32; 3],
    _pad: f32,
}

pub(crate) struct ShaderPreview {
    target: ViewportTarget,
    /// Kept because the target does not report it, and the pipeline needs it.
    format: wgpu::TextureFormat,
    /// Which of `Primitive::CANONICAL` is showing.
    primitive: usize,
    mesh: PreviewMesh,
    pool: MaterialPool,
    textures: MaterialTexturePool,
    /// 🔴 Ours rather than `MaterialPool::bind_group_layout`. That one belongs to the deferred
    /// path: one binding, visible to COMPUTE only — and a *render* pipeline built on it is
    /// invalid, which is exactly how this failed. The contract needs `materials` **and**
    /// `material_values`, both read from the fragment stage.
    materials_bgl: wgpu::BindGroupLayout,
    materials_bg: wgpu::BindGroup,
    camera_buffer: wgpu::Buffer,
    screen_buffer: wgpu::Buffer,
    inti_buffer: wgpu::Buffer,
    frame_bgl: wgpu::BindGroupLayout,
    frame_bg: wgpu::BindGroup,
    /// Groups 1 and 3, which the contract leaves empty and a pipeline layout still has to name.
    empty_bgl: wgpu::BindGroupLayout,
    empty_bg: wgpu::BindGroup,
    /// The pipeline and the shader it was built from, so a graph that has not changed does not
    /// recompile once per frame.
    pipeline: Option<(u64, wgpu::RenderPipeline)>,
    /// Why the last build failed, for the panel to show instead of a black square.
    refusal: Option<String>,
    angle: f32,
}

impl ShaderPreview {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        egui_renderer: &mut egui_wgpu::Renderer,
        format: wgpu::TextureFormat,
    ) -> Self {
        let target = ViewportTarget::new(device, egui_renderer, format, SIZE);
        let primitive = default_primitive();
        let mesh = upload(device, &Primitive::CANONICAL[primitive].1.build());

        let pool = MaterialPool::new(device, &[MaterialParams::default()]);
        let textures = MaterialTexturePool::new(device, queue);

        let uniform = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let camera_buffer = uniform("shader_preview_camera", size_of::<CameraUbo>() as u64);
        let screen_buffer = uniform("shader_preview_screen", size_of::<ScreenUbo>() as u64);
        let inti_buffer = uniform("shader_preview_inti", size_of::<IntiUbo>() as u64);

        let entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let frame_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shader_preview_frame_bgl"),
            entries: &[entry(0), entry(1), entry(2)],
        });
        let frame_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shader_preview_frame_bg"),
            layout: &frame_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: screen_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: inti_buffer.as_entire_binding(),
                },
            ],
        });

        let empty_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shader_preview_empty_bgl"),
            entries: &[],
        });
        let empty_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shader_preview_empty_bg"),
            layout: &empty_bgl,
            entries: &[],
        });

        // Group 2, as the contract declares it: `materials` at 0 and `material_values` at 1.
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let materials_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shader_preview_materials_bgl"),
            entries: &[storage(0), storage(1)],
        });
        let materials_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shader_preview_materials_bg"),
            layout: &materials_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: pool.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: pool.values().as_entire_binding(),
                },
            ],
        });

        Self {
            target,
            format,
            primitive,
            mesh,
            pool,
            textures,
            materials_bgl,
            materials_bg,
            camera_buffer,
            screen_buffer,
            inti_buffer,
            frame_bgl,
            frame_bg,
            empty_bgl,
            empty_bg,
            pipeline: None,
            refusal: None,
            angle: 0.0,
        }
    }

    pub(crate) fn texture_id(&self) -> egui::TextureId {
        self.target.texture_id()
    }

    pub(crate) fn primitive(&self) -> usize {
        self.primitive
    }

    /// Why the shader did not build, if it did not.
    pub(crate) fn refusal(&self) -> Option<&str> {
        self.refusal.as_deref()
    }

    /// Asks for a square target of `side` pixels; applied by [`Self::resize_if_needed`].
    pub(crate) fn request_size(&mut self, side: u32) {
        self.target.request_size((side, side));
    }

    /// Re-creates the target when a new size was asked for — before the UI runs, so the texture id
    /// the panel draws with stays valid for the whole frame.
    pub(crate) fn resize_if_needed(
        &mut self,
        device: &wgpu::Device,
        egui_renderer: &mut egui_wgpu::Renderer,
    ) {
        self.target.resize_if_needed(device, egui_renderer);
    }

    /// Uploads an image a texture node previews with, once per asset.
    pub(crate) fn show_image(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        guid: kooch_core::Guid,
        image: &kooch_render::texture::Image,
    ) {
        if !self.textures.contains(guid) {
            self.textures.register(device, queue, guid, image);
        }
    }

    /// Swaps the shape the shader is shown on.
    pub(crate) fn show_primitive(&mut self, device: &wgpu::Device, index: usize) {
        let index = index.min(Primitive::CANONICAL.len() - 1);
        if index == self.primitive {
            return;
        }
        self.primitive = index;
        self.mesh = upload(device, &Primitive::CANONICAL[index].1.build());
    }

    /// Draws `shader` onto the primitive. `source` and `params_wgsl` are the shader's own, exactly
    /// as the scene's pipelines receive them.
    pub(crate) fn render(
        &mut self,
        gpu: &GpuContext,
        params_wgsl: &str,
        source: &str,
        params: &[ShaderParam],
        // Which image each texture parameter is previewed with, by name.
        images: &[(String, kooch_core::Guid)],
        dt: f32,
    ) {
        let (device, queue) = (gpu.device(), gpu.queue());
        self.angle = (self.angle + dt / TURN_SECONDS).fract();
        if !self.ensure_pipeline(device, params_wgsl, source) {
            return;
        }

        // The camera orbits; the mesh never moves, so the surface's world position is its own.
        let turn = self.angle * std::f32::consts::TAU;
        let eye = Vec3::new(turn.sin() * 2.2, 0.9, turn.cos() * 2.2);
        let view_proj =
            Mat4::perspective_rh(0.9, 1.0, 0.05, 20.0) * Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::bytes_of(&CameraUbo {
                view_proj: view_proj.to_cols_array_2d(),
            }),
        );
        queue.write_buffer(
            &self.screen_buffer,
            0,
            bytemuck::bytes_of(&ScreenUbo {
                material_id: 0,
                mip_bias_scale: 1.0,
                time: self.angle * TURN_SECONDS,
                _pad: 0,
            }),
        );
        queue.write_buffer(
            &self.inti_buffer,
            0,
            bytemuck::bytes_of(&IntiUbo {
                camera_position: eye.to_array(),
                _pad: 0.0,
            }),
        );

        // 🔴 The shader's own starting values, not a material's: a preview shows what the graph
        // declares, before anybody has assigned anything to it.
        let mut values = [0.0f32; MAX_PARAM_SCALARS as usize];
        let mut slots = [TextureRef::default(); 4];
        for param in params {
            if param.kind == ParamKind::Texture {
                // An image that has not reached the pool yet samples the fallback rather than a hole.
                let guid = images
                    .iter()
                    .find(|(name, _)| *name == param.name)
                    .map(|(_, guid)| *guid)
                    .filter(|guid| self.textures.contains(*guid));
                slots[param.offset as usize] = TextureRef {
                    guid,
                    fallback: param.texture,
                };
                continue;
            }
            for component in 0..param.kind.width() {
                let at = (param.offset + component) as usize;
                if at < values.len() {
                    values[at] = param.default[component as usize];
                }
            }
        }
        self.pool.write_values(queue, 0, &values);
        let texture_bg = self.textures.material_bind_group(device, &slots);

        let Some((_, pipeline)) = self.pipeline.as_ref() else {
            return;
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("shader_preview_encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shader_preview_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.target.view(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.05,
                            b: 0.06,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: self.target.depth_view(),
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
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &self.frame_bg, &[]);
            pass.set_bind_group(1, &self.empty_bg, &[]);
            pass.set_bind_group(2, &self.materials_bg, &[]);
            pass.set_bind_group(3, &self.empty_bg, &[]);
            pass.set_bind_group(4, &texture_bg, &[]);
            pass.set_vertex_buffer(0, self.mesh.vertices.slice(..));
            pass.set_index_buffer(self.mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.mesh.count, 0, 0..1);
        }
        queue.submit(Some(encoder.finish()));
    }

    /// Builds the pipeline when the shader changed. Returns whether there is one to draw with.
    fn ensure_pipeline(&mut self, device: &wgpu::Device, params_wgsl: &str, source: &str) -> bool {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        params_wgsl.hash(&mut hasher);
        source.hash(&mut hasher);
        let stamp = hasher.finish();
        if self
            .pipeline
            .as_ref()
            .is_some_and(|(built, _)| *built == stamp)
        {
            return true;
        }

        // 🔴 Checked before it is handed to wgpu: a graph mid-edit produces WGSL that does not
        // compile, and a validation error there is a panic rather than a message.
        if let Err(why) = kooch_render::meshlet::validate_preview(params_wgsl, source) {
            self.refusal = Some(why);
            self.pipeline = None;
            return false;
        }
        self.refusal = None;
        let composed = compose_preview_shader(params_wgsl, source);

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shader_preview_module"),
            source: wgpu::ShaderSource::Wgsl(composed.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shader_preview_layout"),
            bind_group_layouts: &[
                Some(&self.frame_bgl),
                Some(&self.empty_bgl),
                Some(&self.materials_bgl),
                Some(&self.empty_bgl),
                Some(self.textures.layout()),
            ],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shader_preview_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_preview"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: size_of::<PreviewVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3,
                        1 => Float32x3,
                        2 => Float32x2,
                        3 => Float32x4,
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_preview"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: kooch_render::VIEWPORT_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        self.pipeline = Some((stamp, pipeline));
        true
    }
}
