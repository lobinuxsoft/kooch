//! Transparent and masked casters' coverage for the shadow rasters (#1224, #452).
//!
//! 🔴 Baked, not evaluated in the shadow pass: every shadow raster draws all its casters in one
//! material-less draw, and the page cache keeps what it drew. Each transparent material's alpha is
//! rendered over its uv square once a frame instead, and a raster only samples it. What that gives
//! up: an alpha that depends on where the surface is or where it is seen from is taken at a plain
//! upward surface at the origin.

use kooch_core::Guid;

use crate::material::{MaterialPipeline, ShaderKind, SurfaceSource};
use crate::meshlet::{MATERIAL_SURFACE_PRELUDE, ShaderPipelines};

/// Texels a side of one material's coverage.
pub const ALPHA_SIDE: u32 = 128;
/// Transparent or masked materials a frame can shade by alpha; past this they cast solid.
pub const ALPHA_LAYERS: u32 = 32;
/// A layer table entry's mark for a masked material's cut, as `SHADOW_ALPHA_MASKED`.
const MASKED_LAYER: u32 = 1 << 31;
/// Material slots the layer table covers, as the material pool holds.
const MATERIAL_SLOTS: u64 = 256;

/// The bake's own frame, also composed by the geometry trim (#452).
pub(crate) const BAKE_FRAME: &str = include_str!("../../shaders/shadow_alpha_bake.wgsl");
const SAMPLE: &str = include_str!("../../shaders/shadow_alpha.wgsl");

/// The sampling half, with its bind group at `group`.
pub fn shadow_alpha_shader(group: u32) -> String {
    SAMPLE.replace("{{ALPHA_GROUP}}", &group.to_string())
}

/// What one bake draws: the material, and how wide its square is this time — the shadows take
/// [`ALPHA_SIDE`], the geometry trim (#452) its own.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct BakeScreen {
    material_id: u32,
    mip_bias_scale: f32,
    time: f32,
    side: f32,
}

impl BakeScreen {
    pub(crate) fn new(material_id: u32, time: f32, side: f32) -> Self {
        Self {
            material_id,
            mip_bias_scale: 1.0,
            time,
            side,
        }
    }
}

/// The atlas, the table naming each material's layer, and the passes that fill them.
pub struct ShadowAlpha {
    layers: Vec<wgpu::TextureView>,
    table: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    empty_bg: wgpu::BindGroup,
    bake_layout: wgpu::PipelineLayout,
    frame_bgl: wgpu::BindGroupLayout,
    materials_bgl: wgpu::BindGroupLayout,
    screen: wgpu::Buffer,
    screen_stride: u64,
    inti: wgpu::Buffer,
    pipelines: ShaderPipelines<wgpu::RenderPipeline>,
    /// Whether the last bake gave any material a layer: without one, the rasters need no fragment.
    active: bool,
}

impl ShadowAlpha {
    pub fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow_alpha_atlas"),
            size: wgpu::Extent3d {
                width: ALPHA_SIDE,
                height: ALPHA_SIDE,
                depth_or_array_layers: ALPHA_LAYERS,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let atlas = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let layers = (0..ALPHA_LAYERS)
            .map(|layer| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        // Repeat: a tiled uv reads the same coverage the surface shows.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow_alpha_sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let table = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow_alpha_layers"),
            size: MATERIAL_SLOTS * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = Self::bind_group_layout(device);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow_alpha_bg"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&atlas),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: table.as_entire_binding(),
                },
            ],
        });

        let uniform = |binding, dynamic| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: dynamic,
                min_binding_size: None,
            },
            count: None,
        };
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
        let frame_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow_alpha_bake_frame_bgl"),
            entries: &[uniform(0, true), uniform(1, false)],
        });
        let materials_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow_alpha_bake_materials_bgl"),
            entries: &[storage(0), storage(1)],
        });
        let empty = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow_alpha_bake_empty_bgl"),
            entries: &[],
        });
        let textures = crate::material::MaterialTexturePool::bind_group_layout(device);
        let bake_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow_alpha_bake_layout"),
            bind_group_layouts: &[
                Some(&frame_bgl),
                Some(&empty),
                Some(&materials_bgl),
                Some(&empty),
                Some(&textures),
            ],
            immediate_size: 0,
        });
        let align = device.limits().min_uniform_buffer_offset_alignment as u64;
        let screen_stride = align.max(std::mem::size_of::<BakeScreen>() as u64);
        let buffer = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let empty_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow_alpha_bake_empty_bg"),
            layout: &empty,
            entries: &[],
        });
        Self {
            layers,
            table,
            bind_group,
            empty_bg,
            bake_layout,
            frame_bgl,
            materials_bgl,
            screen: buffer("shadow_alpha_screen", screen_stride * ALPHA_LAYERS as u64),
            screen_stride,
            inti: buffer("shadow_alpha_inti", 16),
            pipelines: ShaderPipelines::new(&[
                ShaderKind::Transparent,
                ShaderKind::Surface,
                ShaderKind::Unlit,
            ]),
            active: false,
        }
    }

    /// The layout a shadow raster binds [`Self::bind_group`] with.
    pub fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        let fragment = wgpu::ShaderStages::FRAGMENT;
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow_alpha_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: fragment,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: fragment,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: fragment,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        })
    }

    /// What a raster binds, when any caster shades by alpha this frame.
    pub fn bind_group(&self) -> Option<&wgpu::BindGroup> {
        self.active.then_some(&self.bind_group)
    }

    /// Bakes the coverage of the transparent and masked materials in `slots` and names their
    /// layers. Materials past [`ALPHA_LAYERS`], or whose shader never compiled, cast solid.
    pub fn bake(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        materials: &MaterialPipeline,
        slots: &[u32],
        time: f32,
    ) {
        let mut table = vec![0u32; MATERIAL_SLOTS as usize];
        let chosen = layers_for(slots, |slot| {
            materials
                .slot_surface(slot)
                .is_some_and(|(_, s)| s.kind == ShaderKind::Transparent || s.masked)
        });
        self.active = false;
        if !chosen.is_empty() {
            queue.write_buffer(&self.inti, 0, &[0u8; 16]);
            let materials_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("shadow_alpha_bake_materials_bg"),
                layout: &self.materials_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: materials.pool().buffer().as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: materials.pool().values().as_entire_binding(),
                    },
                ],
            });
            let frame_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("shadow_alpha_bake_frame_bg"),
                layout: &self.frame_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.screen,
                            offset: 0,
                            size: std::num::NonZeroU64::new(
                                std::mem::size_of::<BakeScreen>() as u64
                            ),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.inti.as_entire_binding(),
                    },
                ],
            });
            let empty = &self.empty_bg;
            for (layer, &slot) in chosen.iter().enumerate() {
                let Some((guid, surface)) = materials.slot_surface(slot) else {
                    continue;
                };
                let Some(pipeline) = self.pipeline(device, guid, surface) else {
                    continue;
                };
                queue.write_buffer(
                    &self.screen,
                    layer as u64 * self.screen_stride,
                    bytemuck::bytes_of(&BakeScreen::new(slot, time, ALPHA_SIDE as f32)),
                );
                let textures = materials
                    .texture_pool()
                    .material_bind_group(device, &materials.slot_texture_refs(slot));
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("shadow_alpha_bake"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.layers[layer],
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&pipeline);
                let offset = (layer as u64 * self.screen_stride) as u32;
                pass.set_bind_group(0, &frame_bg, &[offset]);
                pass.set_bind_group(1, empty, &[]);
                pass.set_bind_group(2, &materials_bg, &[]);
                pass.set_bind_group(3, empty, &[]);
                pass.set_bind_group(4, &textures, &[]);
                pass.draw(0..3, 0..1);
                drop(pass);
                table[slot as usize] = layer as u32 + 1;
                if surface.kind != ShaderKind::Transparent {
                    table[slot as usize] |= MASKED_LAYER;
                }
                self.active = true;
            }
        }
        queue.write_buffer(&self.table, 0, bytemuck::cast_slice(&table));
    }

    fn pipeline(
        &self,
        device: &wgpu::Device,
        guid: Guid,
        surface: &SurfaceSource,
    ) -> Option<wgpu::RenderPipeline> {
        self.pipelines.get(guid, surface, false, |surface| {
            let source = [
                MATERIAL_SURFACE_PRELUDE,
                &surface.params_wgsl,
                &surface.source,
                BAKE_FRAME,
            ]
            .join("\n");
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("shadow_alpha_bake"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("shadow_alpha_bake"),
                layout: Some(&self.bake_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_bake"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_bake"),
                    targets: &[Some(wgpu::TextureFormat::R8Unorm.into())],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        })
    }
}

/// The transparent slots a frame bakes, in order, at most [`ALPHA_LAYERS`] of them and each once.
fn layers_for(slots: &[u32], transparent: impl Fn(u32) -> bool) -> Vec<u32> {
    let mut chosen: Vec<u32> = Vec::new();
    for &slot in slots {
        if chosen.len() as u32 == ALPHA_LAYERS {
            break;
        }
        if (slot as u64) < MATERIAL_SLOTS && !chosen.contains(&slot) && transparent(slot) {
            chosen.push(slot);
        }
    }
    chosen
}

#[cfg(test)]
mod tests;
