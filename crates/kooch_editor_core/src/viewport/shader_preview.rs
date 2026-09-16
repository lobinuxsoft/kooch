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
use kooch_render::mesh::{Mesh, MeshVertex, Primitive};
use kooch_render::meshlet::compose_preview_shader;
use wgpu::util::DeviceExt;

use crate::viewport::target::ViewportTarget;

/// How big the preview renders. Fixed: it is a thumbnail of a shader, not a viewport, and a size
/// that follows the panel would rebuild its textures on every drag of the splitter.
const SIZE: (u32, u32) = (320, 320);

/// One turn every this many seconds, so a shader that depends on the view angle shows it.
const TURN_SECONDS: f32 = 12.0;

/// A vertex of the preview mesh. The engine's `MeshVertex` has no tangent, and a normal map cannot
/// be previewed without one, so it is computed here and carried alongside.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct PreviewVertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    tangent: [f32; 4],
}

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

/// The primitive currently uploaded.
struct PreviewMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
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

        Self {
            target,
            format,
            primitive,
            mesh,
            pool,
            textures,
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
                slots[param.offset as usize] = TextureRef {
                    guid: None,
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
            pass.set_bind_group(2, &self.pool.bind_group(device), &[]);
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
                Some(self.pool.layout()),
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

/// Which primitive a preview opens on: the sphere, which is what a material is judged on.
fn default_primitive() -> usize {
    Primitive::CANONICAL
        .iter()
        .position(|(name, _)| *name == "sphere")
        .unwrap_or(0)
}

/// Uploads a primitive, tangents and all.
fn upload(device: &wgpu::Device, mesh: &Mesh) -> PreviewMesh {
    let vertices = with_tangents(mesh);
    PreviewMesh {
        vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shader_preview_vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        }),
        indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shader_preview_indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        }),
        count: mesh.indices.len() as u32,
    }
}

/// The tangent frame the engine's meshes do not carry, accumulated per triangle from the uv
/// derivatives — the standard construction, and what **Unpack Normal** needs to mean anything here.
fn with_tangents(mesh: &Mesh) -> Vec<PreviewVertex> {
    let mut accumulated = vec![Vec3::ZERO; mesh.vertices.len()];
    for triangle in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [triangle[0], triangle[1], triangle[2]].map(|i| i as usize);
        let (va, vb, vc) = (&mesh.vertices[a], &mesh.vertices[b], &mesh.vertices[c]);
        let edge1 = Vec3::from(vb.position) - Vec3::from(va.position);
        let edge2 = Vec3::from(vc.position) - Vec3::from(va.position);
        let duv1 = [vb.uv[0] - va.uv[0], vb.uv[1] - va.uv[1]];
        let duv2 = [vc.uv[0] - va.uv[0], vc.uv[1] - va.uv[1]];
        let determinant = duv1[0] * duv2[1] - duv2[0] * duv1[1];
        // A degenerate uv triangle says nothing about which way the texture runs.
        if determinant.abs() < 1e-12 {
            continue;
        }
        let tangent = (edge1 * duv2[1] - edge2 * duv1[1]) / determinant;
        for index in [a, b, c] {
            accumulated[index] += tangent;
        }
    }

    mesh.vertices
        .iter()
        .zip(accumulated)
        .map(|(vertex, tangent)| {
            let normal = Vec3::from(vertex.normal).normalize_or(Vec3::Y);
            // Gram-Schmidt, so the tangent is square to the normal the shader will use.
            let tangent = (tangent - normal * normal.dot(tangent)).normalize_or(any_square(normal));
            PreviewVertex {
                position: vertex.position,
                normal: normal.to_array(),
                uv: vertex.uv,
                tangent: [tangent.x, tangent.y, tangent.z, 1.0],
            }
        })
        .collect()
}

/// Any direction square to `normal`, for a vertex no triangle gave a tangent.
fn any_square(normal: Vec3) -> Vec3 {
    let axis = if normal.x.abs() < 0.9 {
        Vec3::X
    } else {
        Vec3::Y
    };
    normal.cross(axis).normalize_or(Vec3::X)
}

#[cfg(test)]
mod tests;
