//! Transparency in layers (#452): each pixel keeps its four nearest transparent fragments exactly,
//! and whatever lies behind them blends without order.
//!
//! 🔴 Nothing here is sorted: a 64-bit atomic insertion puts every fragment in its place per pixel,
//! so crossing objects, a mesh seen through itself and two materials inside each other all come
//! out in order, from one raster of both faces. Needs 64-bit atomics that return what they
//! replaced (`SHADER_INT64_ATOMIC_ALL_OPS`); without them the sorted pass draws instead.

mod pipelines;

use bytemuck::{Pod, Zeroable, bytes_of};

use crate::material::ShaderKind;

use super::super::ScreenUbo;
use super::super::shader_cache::ShaderPipelines;
use super::{ForwardFrame, ForwardList, MESHLET_TRIANGLES, Uniforms};
use pipelines::{ACCUM_FORMAT, LAYERS_BINDING, Layouts, OVERFLOW_BINDING, REVEAL_FORMAT};

/// Fragments a pixel keeps exactly.
const LAYERS: u64 = 4;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct DrawArgs {
    vertex_count: u32,
    instance_count: u32,
    first_vertex: u32,
    first_instance: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct CompositeUbo {
    size: [u32; 2],
    _pad: [u32; 2],
}

/// Per-pixel storage, sized to the render resolution it was last drawn at.
struct Targets {
    size: (u32, u32),
    layers: wgpu::Buffer,
    accum: wgpu::TextureView,
    reveal: wgpu::TextureView,
}

pub(super) struct LayeredPass {
    layouts: Layouts,
    insert: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    args_pipeline: wgpu::ComputePipeline,
    tails: ShaderPipelines<wgpu::RenderPipeline>,
    shades: ShaderPipelines<wgpu::ComputePipeline>,
    depth_format: wgpu::TextureFormat,
    targets: Option<Targets>,
    overflow: wgpu::Buffer,
    args: wgpu::Buffer,
    composite_ubo: wgpu::Buffer,
}

impl LayeredPass {
    /// `None` on a device without the 64-bit atomics the insertion reads back.
    pub(super) fn new(
        device: &wgpu::Device,
        depth_format: wgpu::TextureFormat,
        meshlet_bgl: &wgpu::BindGroupLayout,
    ) -> Option<Self> {
        if !device
            .features()
            .contains(wgpu::Features::SHADER_INT64_ATOMIC_ALL_OPS)
        {
            return None;
        }
        let layouts = Layouts::new(device, meshlet_bgl);
        Some(Self {
            insert: pipelines::insert(device, &layouts, depth_format),
            composite: pipelines::composite(device, &layouts),
            args_pipeline: pipelines::args(device, &layouts),
            tails: ShaderPipelines::new(&[ShaderKind::Transparent]),
            shades: ShaderPipelines::new(&[ShaderKind::Transparent]),
            depth_format,
            targets: None,
            overflow: storage(
                device,
                "transparent_overflow",
                16,
                wgpu::BufferUsages::empty(),
            ),
            args: storage(
                device,
                "transparent_args",
                64 * 16,
                wgpu::BufferUsages::INDIRECT,
            ),
            composite_ubo: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("transparent_composite_ubo"),
                size: std::mem::size_of::<CompositeUbo>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            layouts,
        })
    }

    /// Whether the layers for `size` fit one storage binding. At 4K they do not on every device,
    /// and the sorted pass draws instead of failing validation.
    pub(super) fn fits(&self, device: &wgpu::Device, size: (u32, u32)) -> bool {
        layer_bytes(size) <= device.limits().max_storage_buffer_binding_size as u64
    }

    pub(super) fn draw(
        &mut self,
        frame: ForwardFrame<'_>,
        uniforms: &Uniforms,
        list: &ForwardList,
    ) {
        let ForwardFrame {
            device,
            queue,
            encoder,
            ..
        } = frame;
        let size = frame.size;
        if self.targets.as_ref().is_none_or(|t| t.size != size) {
            self.targets = Some(Targets::new(device, size));
        }
        let Some(targets) = self.targets.as_ref() else {
            return;
        };

        // The tail's draws, one per run; emptied on the GPU when nothing overflowed.
        let args: Vec<DrawArgs> = list
            .runs
            .iter()
            .map(|run| DrawArgs {
                vertex_count: MESHLET_TRIANGLES * 3,
                instance_count: run.range.end - run.range.start,
                first_vertex: 0,
                first_instance: run.range.start,
            })
            .collect();
        let args_bytes = (args.len() * std::mem::size_of::<DrawArgs>()) as u64;
        if self.args.size() < args_bytes {
            self.args = storage(
                device,
                "transparent_args",
                args_bytes.next_power_of_two(),
                wgpu::BufferUsages::INDIRECT,
            );
        }
        queue.write_buffer(&self.args, 0, bytemuck::cast_slice(&args));
        queue.write_buffer(
            &self.composite_ubo,
            0,
            bytes_of(&CompositeUbo {
                size: [size.0, size.1],
                _pad: [0; 2],
            }),
        );
        encoder.clear_buffer(&targets.layers, 0, None);
        encoder.clear_buffer(&self.overflow, 0, None);

        let frame_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("transparent_frame_bg"),
            layout: &self.layouts.frame,
            entries: &[
                entry(0, wgpu::BindingResource::TextureView(frame.vbuf)),
                entry(1, uniforms.camera.as_entire_binding()),
                entry(
                    2,
                    wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &uniforms.screen,
                        offset: 0,
                        size: std::num::NonZeroU64::new(std::mem::size_of::<ScreenUbo>() as u64),
                    }),
                ),
                entry(
                    crate::meshlet::MATERIAL_PASS_CONTACT_UBO_BINDING,
                    uniforms.contact.as_entire_binding(),
                ),
                entry(
                    crate::meshlet::MATERIAL_PASS_CONTACT_DEPTH_BINDING,
                    wgpu::BindingResource::TextureView(frame.depth_sample),
                ),
                entry(LAYERS_BINDING, targets.layers.as_entire_binding()),
                entry(OVERFLOW_BINDING, self.overflow.as_entire_binding()),
            ],
        });
        let materials_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("transparent_materials_bg"),
            layout: &self.layouts.materials,
            entries: &[
                entry(0, frame.materials.pool().buffer().as_entire_binding()),
                entry(1, frame.materials.pool().values().as_entire_binding()),
            ],
        });
        let scene_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("transparent_scene_bg"),
            layout: &self.layouts.scene,
            entries: &[
                entry(0, uniforms.list.as_entire_binding()),
                entry(1, frame.scene.instance_buffer().as_entire_binding()),
            ],
        });
        let texture_pool = frame.materials.texture_pool();
        let textures = |material: u32| {
            texture_pool.material_bind_group(device, &frame.materials.slot_texture_refs(material))
        };
        let offset = |material: u32| (material as u64 * uniforms.screen_stride) as u32;
        let first = list.runs[0].material;
        let first_textures = textures(first);
        let depth = || wgpu::RenderPassDepthStencilAttachment {
            view: frame.depth,
            depth_ops: None,
            stencil_ops: None,
        };

        // 1. Every fragment offered to its pixel's four layers.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("transparent_insert"),
                color_attachments: &[],
                depth_stencil_attachment: Some(depth()),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.insert);
            pass.set_bind_group(0, &frame_bg, &[offset(first)]);
            pass.set_bind_group(1, frame.meshlet_bg, &[]);
            pass.set_bind_group(2, &materials_bg, &[]);
            pass.set_bind_group(3, &scene_bg, &[]);
            pass.set_bind_group(4, &first_textures, &[]);
            pass.set_bind_group(5, frame.lights_bg, &[]);
            pass.draw(0..MESHLET_TRIANGLES * 3, 0..list.entries.len() as u32);
        }

        // 2. The tail's draws, emptied when no pixel overflowed.
        {
            let args_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("transparent_args_bg"),
                layout: &self.layouts.args,
                entries: &[
                    entry(0, self.overflow.as_entire_binding()),
                    entry(1, self.args.as_entire_binding()),
                ],
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("transparent_args"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.args_pipeline);
            pass.set_bind_group(0, &args_bg, &[]);
            let slots = (self.args.size() / 16) as u32;
            pass.dispatch_workgroups(slots.div_ceil(64), 1, 1);
        }

        // 3. What lies behind the four, weighted and summed.
        let indirect_offsets = device
            .features()
            .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("transparent_tail"),
                color_attachments: &[
                    Some(clear(&targets.accum, wgpu::Color::TRANSPARENT)),
                    Some(clear(&targets.reveal, wgpu::Color::WHITE)),
                ],
                depth_stencil_attachment: Some(depth()),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(1, frame.meshlet_bg, &[]);
            pass.set_bind_group(2, &materials_bg, &[]);
            pass.set_bind_group(3, &scene_bg, &[]);
            pass.set_bind_group(5, frame.lights_bg, &[]);
            for (index, run) in list.runs.iter().enumerate() {
                let Some((guid, surface)) = frame.materials.slot_surface(run.material) else {
                    continue;
                };
                let Some(pipeline) = self.tails.get(guid, surface, false, |surface| {
                    pipelines::tail(device, &self.layouts, self.depth_format, surface)
                }) else {
                    continue;
                };
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &frame_bg, &[offset(run.material)]);
                pass.set_bind_group(4, &textures(run.material), &[]);
                // A device that cannot start an indirect draw past instance 0 draws the run
                // directly, overflow or not.
                if indirect_offsets {
                    pass.draw_indirect(
                        &self.args,
                        (index * std::mem::size_of::<DrawArgs>()) as u64,
                    );
                } else {
                    pass.draw(0..MESHLET_TRIANGLES * 3, run.range.clone());
                }
            }
        }

        // 4. Each material lights its own layers in place.
        {
            let mut shaded: Vec<u32> = Vec::new();
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("transparent_shade"),
                timestamp_writes: None,
            });
            pass.set_bind_group(1, frame.meshlet_bg, &[]);
            pass.set_bind_group(2, &materials_bg, &[]);
            pass.set_bind_group(3, &scene_bg, &[]);
            pass.set_bind_group(5, frame.lights_bg, &[]);
            for run in &list.runs {
                if shaded.contains(&run.material) {
                    continue;
                }
                shaded.push(run.material);
                let Some((guid, surface)) = frame.materials.slot_surface(run.material) else {
                    continue;
                };
                let Some(pipeline) = self.shades.get(guid, surface, false, |surface| {
                    pipelines::shade(device, &self.layouts, surface)
                }) else {
                    continue;
                };
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &frame_bg, &[offset(run.material)]);
                pass.set_bind_group(4, &textures(run.material), &[]);
                pass.dispatch_workgroups(size.0.div_ceil(8), size.1.div_ceil(8), 1);
            }
        }

        // 5. Layers front to back, then the tail, over the opaque radiance.
        let composite_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("transparent_composite_bg"),
            layout: &self.layouts.composite,
            entries: &[
                entry(0, targets.layers.as_entire_binding()),
                entry(1, wgpu::BindingResource::TextureView(&targets.accum)),
                entry(2, wgpu::BindingResource::TextureView(&targets.reveal)),
                entry(3, self.composite_ubo.as_entire_binding()),
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("transparent_composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: frame.target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.composite);
        pass.set_bind_group(0, &composite_bg, &[]);
        pass.draw(0..3, 0..1);
    }
}

impl Targets {
    fn new(device: &wgpu::Device, size: (u32, u32)) -> Self {
        let texture = |label, format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size.0.max(1),
                        height: size.1.max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        Self {
            size,
            layers: storage(
                device,
                "transparent_layers",
                layer_bytes(size),
                wgpu::BufferUsages::empty(),
            ),
            accum: texture("transparent_accum", ACCUM_FORMAT),
            reveal: texture("transparent_reveal", REVEAL_FORMAT),
        }
    }
}

/// Four 64-bit layers a pixel.
fn layer_bytes(size: (u32, u32)) -> u64 {
    size.0.max(1) as u64 * size.1.max(1) as u64 * LAYERS * 8
}

fn storage(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    extra: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | extra,
        mapped_at_creation: false,
    })
}

fn entry(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource }
}

fn clear(view: &wgpu::TextureView, colour: wgpu::Color) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(colour),
            store: wgpu::StoreOp::Store,
        },
    }
}
