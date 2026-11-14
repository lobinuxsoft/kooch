//! A sky drawn by a `kind: sky` material (#1900): the Shader Graph's own frame, full screen, behind
//! everything. Its parameters and textures land on the groups the scene's shading uses (2 and 4),
//! the way a post-process does, so every node reads what the Inspector assigned.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use kooch_core::Guid;

use crate::VIEWPORT_DEPTH_FORMAT;
use crate::material::shader::MAX_PARAM_TEXTURES;
use crate::material::{
    MaterialPipeline, MaterialPool, MaterialTexturePool, ShaderKind, TextureRef,
};
use crate::meshlet::{compose_sky_shader, validate_sky};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ScreenUbo {
    material_id: u32,
    mip_bias_scale: f32,
    time: f32,
    _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct IntiUbo {
    camera_position: [f32; 3],
    _pad: f32,
}

/// What the camera looks through this frame.
pub(super) struct SkyView {
    pub view: Mat4,
    pub projection: Mat4,
    pub position: Vec3,
    pub time: f32,
}

/// Where a sky reads its parameters and textures: the scene's materials, or the panel's own.
pub(super) struct SkyMaterials<'a> {
    pub pool: &'a MaterialPool,
    pub textures: &'a MaterialTexturePool,
    pub refs: &'a [TextureRef; MAX_PARAM_TEXTURES as usize],
    pub slot: u32,
}

/// The pipeline last built, or why it could not be, and the shader it was built from.
struct Built {
    stamp: u64,
    pipeline: Result<wgpu::RenderPipeline, String>,
}

pub(super) struct SkyGraph {
    format: wgpu::TextureFormat,
    layout: wgpu::PipelineLayout,
    materials_bgl: wgpu::BindGroupLayout,
    frame_bg: wgpu::BindGroup,
    empty_bg: wgpu::BindGroup,
    screen: wgpu::Buffer,
    inti: wgpu::Buffer,
    frame: wgpu::Buffer,
    built: Option<Built>,
    /// The material already complained about, so a frame-rate loop says it once.
    quiet: Option<Guid>,
}

impl SkyGraph {
    pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let buffer = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let screen = buffer("sky_graph_screen", 16);
        let inti = buffer("sky_graph_inti", 16);
        let frame = buffer("sky_graph_frame", 64);
        let frame_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky_graph_frame_bgl"),
            entries: &[uniform(2), uniform(3), uniform(4)],
        });
        let empty_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky_graph_empty_bgl"),
            entries: &[],
        });
        let materials_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky_graph_materials_bgl"),
            entries: &[storage_read(0), storage_read(1)],
        });
        let texture_bgl = MaterialTexturePool::bind_group_layout(device);
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky_graph_layout"),
            bind_group_layouts: &[
                Some(&frame_bgl),
                Some(&empty_bgl),
                Some(&materials_bgl),
                Some(&empty_bgl),
                Some(&texture_bgl),
            ],
            immediate_size: 0,
        });
        let frame_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky_graph_frame_bg"),
            layout: &frame_bgl,
            entries: &[entry(2, &screen), entry(3, &inti), entry(4, &frame)],
        });
        let empty_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky_graph_empty_bg"),
            layout: &empty_bgl,
            entries: &[],
        });
        Self {
            format,
            layout,
            materials_bgl,
            frame_bg,
            empty_bg,
            screen,
            inti,
            frame,
            built: None,
            quiet: None,
        }
    }

    /// Draws `material` into `pass`. `false` when it cannot: not registered, not a sky shader, or
    /// one that does not compile — said once per material, and the pass stays cleared to black.
    pub(super) fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'_>,
        materials: &MaterialPipeline,
        material: Guid,
        view: &SkyView,
    ) -> bool {
        let ready = self.ready(device, materials, material);
        let slot = match ready {
            Ok(slot) => slot,
            Err(why) => {
                if self.quiet != Some(material) {
                    self.quiet = Some(material);
                    tracing::warn!(
                        target: "kooch_render::sky",
                        material = %material,
                        "the sky material drew nothing: {why}",
                    );
                }
                return false;
            }
        };
        if self.quiet == Some(material) {
            self.quiet = None;
        }
        self.record(
            device,
            queue,
            pass,
            SkyMaterials {
                pool: materials.pool(),
                textures: materials.texture_pool(),
                refs: &materials.slot_texture_refs(slot),
                slot,
            },
            view,
        )
    }

    /// The material's slot, with a pipeline built for its current revision.
    fn ready(
        &mut self,
        device: &wgpu::Device,
        materials: &MaterialPipeline,
        material: Guid,
    ) -> Result<u32, String> {
        let slot = materials
            .lookup(material)
            .ok_or("the material is not registered in the MaterialPipeline")?;
        let (_, surface) = materials
            .slot_surface(slot)
            .ok_or("the material names no shader, or its shader has not loaded")?;
        if surface.kind != ShaderKind::Sky {
            return Err(format!("its shader is {:?}, not sky", surface.kind));
        }
        let stamp = (u64::from(slot) << 48) ^ surface.revision;
        self.ensure(device, stamp, &surface.params_wgsl, &surface.source)?;
        Ok(slot)
    }

    /// Builds the pipeline when `stamp` moved. `Err` is why the shader does not compile.
    pub(super) fn ensure(
        &mut self,
        device: &wgpu::Device,
        stamp: u64,
        params_wgsl: &str,
        source: &str,
    ) -> Result<(), String> {
        if let Some(built) = self.built.as_ref()
            && built.stamp == stamp
        {
            return built.pipeline.as_ref().map(|_| ()).map_err(Clone::clone);
        }
        // 🔴 Checked before a pipeline is built from it: a broken edit reads as a message, never
        // as a wgpu validation panic. Once per stamp, so a frame-rate loop says it once.
        let pipeline =
            validate_sky(params_wgsl, source).map(|()| self.build(device, params_wgsl, source));
        let result = pipeline.as_ref().map(|_| ()).map_err(Clone::clone);
        self.built = Some(Built { stamp, pipeline });
        result
    }

    /// Records the built pipeline over `pass`. `false` without one.
    pub(super) fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'_>,
        materials: SkyMaterials<'_>,
        view: &SkyView,
    ) -> bool {
        let Some(Ok(pipeline)) = self.built.as_ref().map(|b| b.pipeline.as_ref()) else {
            return false;
        };
        // The sky has no position: rotation alone turns a pixel into a direction.
        let rotation = Mat4::from_mat3(glam::Mat3::from_mat4(view.view));
        let inverse = (view.projection * rotation).inverse();
        queue.write_buffer(
            &self.screen,
            0,
            bytemuck::bytes_of(&ScreenUbo {
                material_id: materials.slot,
                mip_bias_scale: 1.0,
                time: view.time,
                _pad: 0,
            }),
        );
        queue.write_buffer(
            &self.inti,
            0,
            bytemuck::bytes_of(&IntiUbo {
                camera_position: view.position.to_array(),
                _pad: 0.0,
            }),
        );
        queue.write_buffer(&self.frame, 0, bytemuck::bytes_of(&inverse.to_cols_array()));

        let materials_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky_graph_materials_bg"),
            layout: &self.materials_bgl,
            entries: &[
                entry(0, materials.pool.buffer()),
                entry(1, materials.pool.values()),
            ],
        });
        let textures =
            materials
                .textures
                .material_bind_group(device, materials.slot, materials.refs);
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.frame_bg, &[]);
        pass.set_bind_group(1, &self.empty_bg, &[]);
        pass.set_bind_group(2, &materials_bg, &[]);
        pass.set_bind_group(3, &self.empty_bg, &[]);
        pass.set_bind_group(4, &textures, &[]);
        pass.draw(0..3, 0..1);
        true
    }

    fn build(
        &self,
        device: &wgpu::Device,
        params_wgsl: &str,
        source: &str,
    ) -> wgpu::RenderPipeline {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky_graph"),
            source: wgpu::ShaderSource::Wgsl(compose_sky_shader(params_wgsl, source).into()),
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky_graph"),
            layout: Some(&self.layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_fullscreen"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_sky"),
                targets: &[Some(self.format.into())],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            // The legacy sky's state: the depth it leaves is the clear's, far.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: VIEWPORT_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        })
    }
}

fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

fn uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_read(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
