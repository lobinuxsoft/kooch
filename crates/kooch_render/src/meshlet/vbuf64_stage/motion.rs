//! Where each pixel's surface was last frame (#481, #732 phase 1).
//!
//! The prerequisite every temporal technique shares. TAA needs it; so do
//! FSR 2+, DLSS, XeSS and motion blur, which is why one pass unblocks
//! four features rather than one.
//!
//! # Its own pass, at full resolution
//!
//! Shading runs at half rate (#825) and a temporal resolve wants a
//! vector per pixel, not per 2x2 quad. This is the cheap half of the
//! surface reconstruction — positions only, no normals, no tangents, no
//! texture sampling — so paying for it at full resolution costs a
//! fraction of what shading there would.
//!
//! It also runs whether or not the shading path is the compute one: the
//! vector is a property of the geometry and the camera, not of how the
//! pixel was lit.

use bytemuck::{Pod, Zeroable};

use crate::meshlet::material_pass::{SURFACE_RECONSTRUCT_SHADER, VISIBILITY_BUFFER_RESOLVE_SHADER};

use super::{CameraUbo, ScreenUbo, VBUF64_FORMAT};

const MOTION_BODY: &str = include_str!("../../../shaders/motion_vectors.wgsl");

/// Two channels of half float: the vector is a UV offset in -1..1 and
/// nothing downstream wants more range or more precision than that.
/// Bevy's prepass uses the same format for the same reason.
pub const MOTION_VECTOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;

/// The group the motion pass adds. 0, 1 and 3 are taken by the
/// concatenated prefix — the vbuf and frame uniforms, the mesh pool, and
/// the visible-meshlet plus instance buffers.
const MOTION_GROUP: u32 = 2;

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
struct MotionUbo {
    clip_from_world: [[f32; 4]; 4],
    previous_clip_from_world: [[f32; 4]; 4],
}

pub(super) struct MotionVectors {
    pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    // 🔴 Its own group 0 rather than the shading pass's. That one also
    // carries the colour target, the shaded-id target and the
    // contact-shadow pair — bindings this shader never declares, and a
    // layout with entries the module does not use is rejected.
    frame_bgl: wgpu::BindGroupLayout,
    scene_bgl: wgpu::BindGroupLayout,
    camera_ubo: wgpu::Buffer,
    screen_ubo: wgpu::Buffer,
    ubo: wgpu::Buffer,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// Last frame's view-projection, kept per stage because each view
    /// has its own camera. A shared one would describe the Game panel's
    /// movement in the editor viewport's motion vectors.
    ///
    /// `None` on the first frame: there is no previous camera, so every
    /// vector would be measured against a matrix of zeros.
    ///
    /// Behind a lock because the whole render chain is on `&self` — the
    /// same reason the debug pipelines are behind a `OnceLock` — and the
    /// stage lives in `Resources`, which requires `Sync`. A `Cell` is
    /// not. Taken once per frame per view, so there is nothing to
    /// contend for.
    previous_view_proj: std::sync::Mutex<Option<glam::Mat4>>,
}

impl MotionVectors {
    pub(super) fn new(
        device: &wgpu::Device,
        size: (u32, u32),
        pool_bgl: &wgpu::BindGroupLayout,
    ) -> Self {
        let source = [
            VISIBILITY_BUFFER_RESOLVE_SHADER,
            SURFACE_RECONSTRUCT_SHADER,
            MOTION_BODY,
        ]
        .join("\n");
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("motion_vectors_shader"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });

        let uniform = |binding: u32, min: u64| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: std::num::NonZeroU64::new(min),
            },
            count: None,
        };
        let storage = |binding: u32| wgpu::BindGroupLayoutEntry {
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
            label: Some("motion_vectors_frame_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::ReadOnly,
                        format: VBUF64_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                uniform(1, std::mem::size_of::<CameraUbo>() as u64),
                uniform(2, std::mem::size_of::<ScreenUbo>() as u64),
            ],
        });
        let scene_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("motion_vectors_scene_bgl"),
            entries: &[storage(0), storage(1)],
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("motion_vectors_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let mut layouts = [None, None, None, None];
        layouts[0] = Some(&frame_bgl);
        layouts[1] = Some(pool_bgl);
        layouts[MOTION_GROUP as usize] = Some(&bgl);
        layouts[3] = Some(&scene_bgl);
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("motion_vectors_layout"),
            bind_group_layouts: &layouts,
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("motion_vectors_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_motion_vectors"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_motion_vectors"),
                targets: &[Some(MOTION_VECTOR_FORMAT.into())],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let small = |label: &'static str, size: u64| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let camera_ubo = small(
            "motion_vectors_camera",
            std::mem::size_of::<CameraUbo>() as u64,
        );
        let screen_ubo = small(
            "motion_vectors_screen",
            std::mem::size_of::<ScreenUbo>() as u64,
        );
        let ubo = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("motion_vectors_ubo"),
            size: std::mem::size_of::<MotionUbo>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (texture, view) = create_target(device, size);
        Self {
            pipeline,
            bgl,
            frame_bgl,
            scene_bgl,
            camera_ubo,
            screen_ubo,
            ubo,
            texture,
            view,
            previous_view_proj: std::sync::Mutex::new(None),
        }
    }

    pub(super) fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        let (texture, view) = create_target(device, size);
        self.texture = texture;
        self.view = view;
        // The history is a camera matrix, not a texture, so a resize does
        // not invalidate it — but the vectors it produces are in UV
        // space, and UV space just changed shape. One frame of zeros is
        // cheaper than one frame of a temporal pass reprojecting from a
        // different aspect ratio.
        *self.previous_view_proj.lock().expect("motion history lock") = None;
    }

    pub(super) fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub(super) fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    /// Records the pass and remembers `view_proj` for the next frame.
    ///
    /// ⚠️ `view_proj` must be the **unjittered** matrix. Sub-pixel jitter
    /// is what a temporal resolve accumulates; a motion vector carrying
    /// it would describe the jitter as scene motion and the reprojection
    /// would cancel exactly the signal being integrated.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn dispatch(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        vbuf_view: &wgpu::TextureView,
        meshlet_bg: &wgpu::BindGroup,
        visible_meshlets: &wgpu::Buffer,
        instances: &wgpu::Buffer,
        previous_transforms: &wgpu::Buffer,
        view_proj: glam::Mat4,
        size: (u32, u32),
    ) {
        // 🔴 The first frame reprojects against itself, which is a vector
        // of zero everywhere. The alternative is a matrix of zeros, whose
        // `w` divide is a division by zero — NaNs into a texture a
        // temporal pass will read for as long as its history survives.
        let mut history = self.previous_view_proj.lock().expect("motion history lock");
        let previous = history.unwrap_or(view_proj);
        queue.write_buffer(
            &self.ubo,
            0,
            bytemuck::bytes_of(&MotionUbo {
                clip_from_world: view_proj.to_cols_array_2d(),
                previous_clip_from_world: previous.to_cols_array_2d(),
            }),
        );
        queue.write_buffer(
            &self.camera_ubo,
            0,
            bytemuck::bytes_of(&CameraUbo {
                view_proj: view_proj.to_cols_array_2d(),
            }),
        );
        queue.write_buffer(
            &self.screen_ubo,
            0,
            bytemuck::bytes_of(&ScreenUbo {
                size: [size.0, size.1],
                material_id: 0,
                debug_mode: 0,
                // Full rate, always: a temporal resolve wants a vector
                // per pixel. `screen.shading_rate` only reaches the
                // reconstruction through `frag_coord_to_ndc`, which this
                // pass calls with real pixel centres.
                shading_rate: 1,
                _pad: [0; 3],
            }),
        );

        let frame_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("motion_vectors_frame_bg"),
            layout: &self.frame_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(vbuf_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.camera_ubo.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.screen_ubo.as_entire_binding(),
                },
            ],
        });
        let scene_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("motion_vectors_scene_bg"),
            layout: &self.scene_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: visible_meshlets.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: instances.as_entire_binding(),
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("motion_vectors_bind_group"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: previous_transforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.ubo.as_entire_binding(),
                },
            ],
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("motion_vectors_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Every pixel is written, background included, so
                        // nothing is preserved.
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &frame_bg, &[]);
            pass.set_bind_group(1, meshlet_bg, &[]);
            pass.set_bind_group(MOTION_GROUP, &bind_group, &[]);
            pass.set_bind_group(3, &scene_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        *history = Some(view_proj);
    }
}

fn create_target(device: &wgpu::Device, size: (u32, u32)) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("motion_vectors"),
        size: wgpu::Extent3d {
            width: size.0.max(1),
            height: size.1.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: MOTION_VECTOR_FORMAT,
        // COPY_SRC so a test can read the vectors back; the temporal
        // passes that will consume them sample rather than copy.
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}
