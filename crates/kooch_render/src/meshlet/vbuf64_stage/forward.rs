//! Transparent surfaces over the opaque scene (#452): in layers where the device allows, sorted far
//! to near where it does not.
//!
//! 🔴 Between the shading and the temporal resolve, on the render-resolution radiance: the glass
//! is lit in the same units as what it covers and goes through the same upscale and tonemap. It
//! tests the depth the raster wrote and never writes it, so what is behind glass stays reachable
//! for the contact march and the Hi-Z.

mod layered;
mod list;

use bytemuck::bytes_of;

use crate::contact_shadow::ContactShadowUbo;
use crate::material::{MaterialPipeline, ShaderKind};
use crate::meshlet::deferred::HDR_COLOR_FORMAT;
use crate::meshlet::scene::MeshletScene;
use crate::meshlet::{
    MATERIAL_FORWARD_FRAME, MATERIAL_PASS_CONTACT_DEPTH_BINDING, MATERIAL_PASS_CONTACT_UBO_BINDING,
    compose_material_shader,
};

use super::shader_cache::ShaderPipelines;
use super::two_pass::SharedLayouts;
use super::{CameraUbo, ScreenUbo};

pub(crate) use list::ForwardList;

/// Triangles a meshlet can hold: its triangle index is 7 bits wherever it is packed. Every draw
/// instance emits this many, and the vertex shader drops the ones past its own meshlet's count.
pub(super) const MESHLET_TRIANGLES: u32 = 128;

/// Screen slots, one per material, as the fragment path has.
const MAX_SLOTS: u64 = 256;

/// What the frame hands the pass besides its list.
pub(super) struct ForwardFrame<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// Linear radiance at render resolution, blended into.
    pub target: &'a wgpu::TextureView,
    pub depth: &'a wgpu::TextureView,
    pub depth_sample: &'a wgpu::TextureView,
    pub vbuf: &'a wgpu::TextureView,
    pub meshlet_bg: &'a wgpu::BindGroup,
    pub scene: &'a MeshletScene,
    pub materials: &'a MaterialPipeline,
    pub lights_bg: &'a wgpu::BindGroup,
    pub view_proj: glam::Mat4,
    pub contact: &'a ContactShadowUbo,
    pub size: (u32, u32),
    pub mip_bias_scale: f32,
    pub time: f32,
}

/// What every transparent pass reads: the camera, one screen slot per material, the contact march's
/// settings and the list.
pub(super) struct Uniforms {
    list: wgpu::Buffer,
    camera: wgpu::Buffer,
    screen: wgpu::Buffer,
    screen_stride: u64,
    contact: wgpu::Buffer,
}

impl Uniforms {
    fn new(device: &wgpu::Device) -> Self {
        let uniform = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let align = device.limits().min_uniform_buffer_offset_alignment as u64;
        let screen_stride = align.max(std::mem::size_of::<ScreenUbo>() as u64);
        Self {
            list: list_buffer(device, 256),
            camera: uniform("forward_camera", std::mem::size_of::<CameraUbo>() as u64),
            screen: uniform("forward_screen", screen_stride * MAX_SLOTS),
            screen_stride,
            contact: uniform(
                "forward_contact",
                std::mem::size_of::<ContactShadowUbo>() as u64,
            ),
        }
    }

    fn write(&mut self, frame: &ForwardFrame<'_>, list: &ForwardList) {
        let bytes = (list.entries.len() * 4) as u64;
        if self.list.size() < bytes {
            self.list = list_buffer(frame.device, bytes.next_power_of_two());
        }
        let queue = frame.queue;
        queue.write_buffer(&self.list, 0, bytemuck::cast_slice(&list.entries));
        queue.write_buffer(
            &self.camera,
            0,
            bytes_of(&CameraUbo {
                view_proj: frame.view_proj.to_cols_array_2d(),
            }),
        );
        queue.write_buffer(&self.contact, 0, bytes_of(frame.contact));
        for run in &list.runs {
            queue.write_buffer(
                &self.screen,
                run.material as u64 * self.screen_stride,
                bytes_of(&ScreenUbo {
                    size: [frame.size.0, frame.size.1],
                    material_id: run.material,
                    debug_mode: 0,
                    shading_rate: 1,
                    mip_bias_scale: frame.mip_bias_scale,
                    time: frame.time,
                    _pad: [0; 1],
                }),
            );
        }
    }
}

pub(super) struct ForwardPass {
    uniforms: Uniforms,
    layered: Option<layered::LayeredPass>,
    sorted: SortedPass,
}

impl ForwardPass {
    pub(super) fn new(
        device: &wgpu::Device,
        depth_format: wgpu::TextureFormat,
        meshlet_bgl: &wgpu::BindGroupLayout,
        shared: SharedLayouts<'_>,
    ) -> Self {
        Self {
            uniforms: Uniforms::new(device),
            layered: layered::LayeredPass::new(device, depth_format, meshlet_bgl),
            sorted: SortedPass::new(device, depth_format, meshlet_bgl, shared),
        }
    }

    /// Draws `list` over the frame. Costs nothing when it is empty: no buffer written, no pass.
    pub(super) fn draw(
        &mut self,
        frame: ForwardFrame<'_>,
        layouts: SharedLayouts<'_>,
        list: &ForwardList,
    ) {
        // A material past the screen slots has nowhere to put its uniforms; it is left out.
        let mut list = std::borrow::Cow::Borrowed(list);
        if list.runs.iter().any(|run| run.material as u64 >= MAX_SLOTS) {
            list.to_mut()
                .runs
                .retain(|run| (run.material as u64) < MAX_SLOTS);
        }
        if list.runs.is_empty() {
            return;
        }
        self.uniforms.write(&frame, &list);
        match self.layered.as_mut() {
            Some(layered) if layered.fits(frame.device, frame.size) => {
                layered.draw(frame, &self.uniforms, &list)
            }
            _ => self.sorted.draw(frame, layouts, &self.uniforms, &list),
        }
    }
}

/// The fallback: each transparent instance far to near, blended as it lands. Two crossing objects,
/// or a mesh seen through itself, can come out in the wrong order; back faces are culled.
struct SortedPass {
    pipelines: ShaderPipelines<wgpu::RenderPipeline>,
    /// The raster's, which this pass tests against.
    depth_format: wgpu::TextureFormat,
    /// The two-pass frame's groups, except the scene: its vertex stage reads the list too.
    layout: wgpu::PipelineLayout,
    scene_bgl: wgpu::BindGroupLayout,
}

impl SortedPass {
    fn new(
        device: &wgpu::Device,
        depth_format: wgpu::TextureFormat,
        meshlet_bgl: &wgpu::BindGroupLayout,
        shared: SharedLayouts<'_>,
    ) -> Self {
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let scene_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("forward_scene_bgl"),
            entries: &[storage(0), storage(1)],
        });
        let textures = crate::material::MaterialTexturePool::bind_group_layout(device);
        let lights = kooch_lighting::GpuLights::bind_group_layout(device);
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("forward_layout"),
            bind_group_layouts: &[
                Some(shared.frame),
                Some(meshlet_bgl),
                Some(shared.materials),
                Some(&scene_bgl),
                Some(&textures),
                Some(&lights),
            ],
            immediate_size: 0,
        });
        Self {
            pipelines: ShaderPipelines::new(&[ShaderKind::Transparent]),
            depth_format,
            layout,
            scene_bgl,
        }
    }

    fn draw(
        &mut self,
        frame: ForwardFrame<'_>,
        layouts: SharedLayouts<'_>,
        uniforms: &Uniforms,
        list: &ForwardList,
    ) {
        let ForwardFrame {
            device, encoder, ..
        } = frame;
        let frame_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("forward_frame_bg"),
            layout: layouts.frame,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(frame.vbuf),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniforms.camera.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &uniforms.screen,
                        offset: 0,
                        size: std::num::NonZeroU64::new(std::mem::size_of::<ScreenUbo>() as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: MATERIAL_PASS_CONTACT_UBO_BINDING,
                    resource: uniforms.contact.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: MATERIAL_PASS_CONTACT_DEPTH_BINDING,
                    resource: wgpu::BindingResource::TextureView(frame.depth_sample),
                },
            ],
        });
        let materials_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("forward_materials_bg"),
            layout: layouts.materials,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: frame.materials.pool().buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: frame.materials.pool().values().as_entire_binding(),
                },
            ],
        });
        let scene_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("forward_scene_bg"),
            layout: &self.scene_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.list.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: frame.scene.instance_buffer().as_entire_binding(),
                },
            ],
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("forward_transparent"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: frame.target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            // Read-only, so the same depth is also sampled by the contact march.
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: frame.depth,
                depth_ops: None,
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(1, frame.meshlet_bg, &[]);
        pass.set_bind_group(2, &materials_bg, &[]);
        pass.set_bind_group(3, &scene_bg, &[]);
        pass.set_bind_group(5, frame.lights_bg, &[]);
        let texture_pool = frame.materials.texture_pool();
        for run in &list.runs {
            let Some((guid, surface)) = frame.materials.slot_surface(run.material) else {
                continue;
            };
            let Some(pipeline) = self.pipelines.get(guid, surface, false, |surface| {
                build_pipeline(device, &self.layout, self.depth_format, surface)
            }) else {
                continue;
            };
            let refs = frame.materials.slot_texture_refs(run.material);
            let textures = texture_pool.material_bind_group(device, &refs);
            pass.set_pipeline(&pipeline);
            let offset = (run.material as u64 * uniforms.screen_stride) as u32;
            pass.set_bind_group(0, &frame_bg, &[offset]);
            pass.set_bind_group(4, &textures, &[]);
            pass.draw(0..MESHLET_TRIANGLES * 3, run.range.clone());
        }
    }
}

fn list_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("forward_list"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn build_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    depth_format: wgpu::TextureFormat,
    surface: &crate::material::SurfaceSource,
) -> wgpu::RenderPipeline {
    let source = compose_material_shader(
        MATERIAL_FORWARD_FRAME,
        &surface.params_wgsl,
        &surface.source,
        false,
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("forward_transparent"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("forward_transparent"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_forward"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_forward"),
            targets: &[Some(wgpu::ColorTargetState {
                format: HDR_COLOR_FORMAT,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::COLOR,
            })],
            compilation_options: Default::default(),
        }),
        // Back faces culled, as the opaque raster does: a closed glass shape then blends its near
        // side only, which needs no sorting inside the mesh.
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: depth_format,
            depth_write_enabled: Some(false),
            // Reversed-Z, as the raster that wrote it.
            depth_compare: Some(wgpu::CompareFunction::Greater),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests;
