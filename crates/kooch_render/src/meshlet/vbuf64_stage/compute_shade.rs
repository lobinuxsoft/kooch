//! Compute shading over the visibility buffer (#824) — the tile's light
//! list, read once.
//!
//! The alternative to [`MaterialTwoPass`](super::two_pass::MaterialTwoPass),
//! not a replacement for it: both stay wired, and
//! [`ComputeShading::enabled`] decides which one a frame takes. Two
//! paths that shade the same vbuf is what makes the capture pair the
//! issue asks for possible on the device, with one environment variable
//! and no rebuild.
//!
//! # What differs from the fragment path
//!
//! Everything above the entry point is the same composed shader, so the
//! shading model, the reconstruction and the material sampling are
//! identical by construction. Three things change:
//!
//! - **A workgroup owns a 16x16 tile.** It reduces its threads' froxels
//!   to one block of cells, copies that block's light *indices* into
//!   `var<workgroup>` memory, and every thread then walks its own cell's
//!   run out of shared memory.
//!
//!   🔴 The INDICES, and only those — four bytes each. Every thread
//!   still fetches the whole 80-byte `IntiLight` out of the storage
//!   buffer for every light in its froxel, because that is what
//!   `inti_lights[tile_lights[i]]` reads. Fifteen of those is 1.2 KB a
//!   pixel; this pass removed the 60 bytes of indices and left the
//!   1200, which is why it measured 6.6 % on the device and not more.
//!   Moving the records themselves is #826's, and `ROADMAP.md` carries
//!   the numbers.
//! - **No material-depth target.** The fragment path resolves
//!   `material_id` into a depth buffer and lets a hardware `Equal` test
//!   do the per-material cull. A compute pass has no depth test, so each
//!   thread resolves its own pixel's material — two dependent reads,
//!   short of the full reconstruction, which only the pixels a dispatch
//!   owns pay for.
//! - **The colour target is cleared once**, up front, instead of by the
//!   first material's `LoadOp::Clear`. Nothing else can clear it: a
//!   compute dispatch writes the pixels it owns and no others.

use bytemuck::bytes_of;

use crate::contact_shadow::ContactShadowUbo;
use crate::material::{MaterialPipeline, MaterialTexturePool};
use crate::meshlet::dispatcher::MeshletCull;
use crate::meshlet::scene::MeshletScene;
use crate::meshlet::{
    MATERIAL_PASS_CONTACT_DEPTH_BINDING, MATERIAL_PASS_CONTACT_UBO_BINDING,
    MATERIAL_PBR_COMPUTE_BODY, SHADING_TILE_SIZE, compose_material_shader,
};

use super::upsample::SHADED_ID_FORMAT;
use super::{CameraUbo, ScreenUbo, ShadingRate, VBUF64_FORMAT};
use crate::meshlet::deferred::HDR_COLOR_FORMAT;

/// Upper bound on shading slots the per-frame screen UBO can address.
/// Matches the fragment path's, and `MaterialPipeline::DEFAULT_CAPACITY`.
const MAX_SHADING_SLOTS: u32 = 256;

/// Group-0 binding of the shading target. The vbuf, camera, screen and
/// the contact-shadow pair hold 0..4; this is the next free index and
/// the shader declares it at the same one.
const COLOR_OUT_BINDING: u32 = 5;

/// Group-0 binding of the per-sample surface id (#825). Written only at
/// reduced rate, bound always: a bind group layout cannot depend on a
/// setting the player changes between frames.
const SHADED_IDS_BINDING: u32 = 6;

/// `KOOCH_COMPUTE_SHADING=on` (or `1`, or `true`), read once.
///
/// 🔴 **Off by default, and an environment variable rather than only an
/// editor control.** Both halves matter and for different reasons.
///
/// Off, because the fragment path is the one three sessions of captures
/// were taken against. A new shading path that is one bit different
/// from it would change every baseline at once, and the way to find out
/// whether it is different is to run both — not to ship one.
///
/// An environment variable, because **the editor is not where this can
/// be measured**. On the desktop GPU the whole raster pass is 0.12 ms;
/// deleting the entire specular model moved it by 0.001 (#821). The
/// frame this exists for is a game on the OneXFly launched through
/// Steam, with no editor in the process. `KOOCH_CLUSTERING` and
/// `KOOCH_SPECULAR_FLOOR` already carried that lesson; this is the
/// third time it has been the same lesson.
///
/// Anything unrecognised is `None`, the same as unset: a typo during a
/// measurement run must not silently change which path is being
/// measured, and — since the settings asset now supplies a value too
/// (#830) — must not silently override the author's either.
pub(crate) fn enabled_by_environment() -> Option<bool> {
    static ON: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = parse_enabled(std::env::var("KOOCH_COMPUTE_SHADING").ok().as_deref());
        if on == Some(true) {
            tracing::info!(
                target: "kooch_render::vbuf64_stage",
                "KOOCH_COMPUTE_SHADING=on: shading runs as a compute pass over the \
                 visibility buffer, with each tile's froxel lights in workgroup memory",
            );
        }
        on
    })
}

/// The parse, apart from the read, so a test can exercise it without
/// touching the process environment. Three tests fighting over one
/// variable is a bug this repository has already shipped twice
/// (`KOOCH_PACK_KEY`, `KOOCH_ENGINE_HOME`) and a pure function cannot
/// have it.
///
/// `None` means "the variable said nothing", which is what lets the
/// project's own setting stand.
fn parse_enabled(raw: Option<&str>) -> Option<bool> {
    match raw {
        Some("on") | Some("1") | Some("true") => Some(true),
        Some("off") | Some("0") | Some("false") => Some(false),
        _ => None,
    }
}

fn build_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    debug: bool,
) -> wgpu::ComputePipeline {
    let src = compose_material_shader(MATERIAL_PBR_COMPUTE_BODY, debug);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(if debug {
            "material_pbr_compute_shader_debug"
        } else {
            "material_pbr_compute_shader"
        }),
        source: wgpu::ShaderSource::Wgsl(src.into()),
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("compute_shading_pipeline"),
        layout: Some(layout),
        module: &module,
        entry_point: Some("cs_shade_tile"),
        compilation_options: Default::default(),
        cache: None,
    })
}

pub(super) struct ComputeShading {
    pipeline: wgpu::ComputePipeline,
    /// The same pipeline with the debug views concatenated in (#743).
    /// Built on the frame somebody first opens one, for the reasons
    /// `MaterialTwoPass` documents at length: a shipped game compiles
    /// neither this nor a byte of what is inside it.
    pipeline_debug: std::sync::OnceLock<wgpu::ComputePipeline>,
    layout: wgpu::PipelineLayout,
    frame_bgl: wgpu::BindGroupLayout,
    materials_bgl: wgpu::BindGroupLayout,
    scene_bgl: wgpu::BindGroupLayout,
    camera_buffer: wgpu::Buffer,
    screen_buffer: wgpu::Buffer,
    screen_stride: u64,
    contact_buffer: wgpu::Buffer,
}

impl ComputeShading {
    pub(super) fn new(device: &wgpu::Device, meshlet_bgl: &wgpu::BindGroupLayout) -> Self {
        let storage_read = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let uniform = |binding: u32, dynamic: bool, min: u64| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: dynamic,
                min_binding_size: std::num::NonZeroU64::new(min),
            },
            count: None,
        };

        // Group 0: vbuf(read) + camera + dynamic-offset screen + the
        // contact-shadow pair + the shading target.
        let frame_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("compute_shading_frame_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::ReadOnly,
                        format: VBUF64_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                uniform(1, false, 64),
                uniform(2, true, std::mem::size_of::<ScreenUbo>() as u64),
                uniform(
                    MATERIAL_PASS_CONTACT_UBO_BINDING,
                    false,
                    std::mem::size_of::<ContactShadowUbo>() as u64,
                ),
                wgpu::BindGroupLayoutEntry {
                    binding: MATERIAL_PASS_CONTACT_DEPTH_BINDING,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: COLOR_OUT_BINDING,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        // Linear radiance, not a picture (#732). The
                        // tonemap is a pass of its own so TAA can sit
                        // between the two.
                        format: HDR_COLOR_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: SHADED_IDS_BINDING,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: SHADED_ID_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let materials_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("compute_shading_materials_bgl"),
            entries: &[storage_read(0)],
        });
        let scene_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("compute_shading_scene_bgl"),
            entries: &[storage_read(0), storage_read(1)],
        });
        let texture_bgl = MaterialTexturePool::bind_group_layout(device);
        let lights_bgl = kooch_lighting::GpuLights::bind_group_layout(device);

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("compute_shading_layout"),
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
        let pipeline = build_pipeline(device, &layout, false);

        let align = device.limits().min_uniform_buffer_offset_alignment as u64;
        let screen_stride = align.max(std::mem::size_of::<ScreenUbo>() as u64);
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("compute_shading_camera_ubo"),
            size: std::mem::size_of::<CameraUbo>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let screen_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("compute_shading_screen_ubo"),
            size: screen_stride * MAX_SHADING_SLOTS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let contact_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("compute_shading_contact_shadow_ubo"),
            size: std::mem::size_of::<ContactShadowUbo>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            pipeline_debug: std::sync::OnceLock::new(),
            layout,
            frame_bgl,
            materials_bgl,
            scene_bgl,
            camera_buffer,
            screen_buffer,
            screen_stride,
            contact_buffer,
        }
    }

    fn pipeline_for(&self, device: &wgpu::Device, debug_mode: u32) -> &wgpu::ComputePipeline {
        if debug_mode == 0 {
            return &self.pipeline;
        }
        self.pipeline_debug
            .get_or_init(|| build_pipeline(device, &self.layout, true))
    }

    /// One dispatch per registered shading slot, each over the whole
    /// shaded target in 16x16 tiles.
    ///
    /// Whole-target and not "the tiles this material covers": which
    /// tiles those are is not known without a classification pass. A
    /// tile with none of this material's pixels does no reconstruction,
    /// reads no lights and writes nothing — but it does not leave for
    /// free either. Every thread of it still pays:
    ///
    /// - one `textureLoad` of the R64 vbuf,
    /// - `visible_meshlets[slot]`, which depends on that load,
    /// - `instances[inst_id].material_id`, which depends on that one,
    /// - and the three `workgroupBarrier`s, which are unconditional.
    ///
    /// Two dependent storage reads, which is the access pattern the
    /// device measurements left standing after ALU was ruled out.
    ///
    /// 🔴 And the count is not "materials in the scene". [`shading_slots`]
    /// is `0..next_slot`, and `sync_from_resources` registers every
    /// material the `AssetDatabase` knows about — so dropping an unused
    /// `.ron` into the project's folder adds a full-screen sweep to
    /// every frame. Compacting the dispatch is a change to this function
    /// alone, and `KOOCH_SHADING_PAD` is what says whether it is worth
    /// making: it appends sweeps that own no pixel, so an A/B across it
    /// prices exactly this paragraph (#885).
    ///
    /// [`shading_slots`]: crate::material::MaterialPipeline::shading_slots
    ///
    /// `color_view` and `ids_view` are the targets for the given `rate`:
    /// the screen at [`ShadingRate::Full`], the half-resolution pair at
    /// [`ShadingRate::Half`]. `screen_size` stays the full resolution on
    /// both — the tile still covers `rate` times as many pixels, and
    /// every coordinate the shading model uses is a screen coordinate.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn shade(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        vbuf_view: &wgpu::TextureView,
        depth_sample_view: &wgpu::TextureView,
        color_view: &wgpu::TextureView,
        ids_view: &wgpu::TextureView,
        meshlet_bg: &wgpu::BindGroup,
        cull: &MeshletCull,
        scene: &MeshletScene,
        material_pipeline: &MaterialPipeline,
        lights_bg: &wgpu::BindGroup,
        view_proj: glam::Mat4,
        contact: &ContactShadowUbo,
        screen_size: (u32, u32),
        rate: ShadingRate,
        // `exp2(mip_bias)`; see `ScreenUbo::mip_bias_scale` (#881).
        mip_bias_scale: f32,
        debug_mode: u32,
    ) {
        let pipeline = self.pipeline_for(device, debug_mode);

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
        // `KOOCH_SHADING_PAD`, applied here so the loop below writes a
        // `ScreenUbo` for the padded slots too. Zero unless a
        // measurement run asked for it (#885).
        let slots = super::shading_pad::padded_slots(slots, MAX_SHADING_SLOTS);
        for slot in slots.clone() {
            queue.write_buffer(
                &self.screen_buffer,
                slot as u64 * self.screen_stride,
                bytes_of(&ScreenUbo {
                    size: [screen_size.0, screen_size.1],
                    material_id: slot,
                    debug_mode,
                    shading_rate: rate.factor(),
                    mip_bias_scale,
                    _pad: [0; 2],
                }),
            );
        }

        // The background. A dispatch writes only the pixels it owns, so
        // without this the previous frame's colour survives wherever
        // nothing is drawn — and the blit composes this target over the
        // sky expecting alpha to say what was covered.
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("compute_shading_clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        let frame_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("compute_shading_frame_bg"),
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
                wgpu::BindGroupEntry {
                    binding: COLOR_OUT_BINDING,
                    resource: wgpu::BindingResource::TextureView(color_view),
                },
                wgpu::BindGroupEntry {
                    binding: SHADED_IDS_BINDING,
                    resource: wgpu::BindingResource::TextureView(ids_view),
                },
            ],
        });
        let materials_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("compute_shading_materials_bg"),
            layout: &self.materials_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: material_pipeline.pool().buffer().as_entire_binding(),
            }],
        });
        let scene_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("compute_shading_scene_bg"),
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

        // The dispatch covers the shaded target, not the screen: at half
        // rate that is a quarter of the threads, which is the entire
        // point of #825.
        let shaded_size = rate.target_size(screen_size);
        let tiles_x = shaded_size.0.div_ceil(SHADING_TILE_SIZE);
        let tiles_y = shaded_size.1.div_ceil(SHADING_TILE_SIZE);
        let texture_pool = material_pipeline.texture_pool();
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("compute_shading_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(1, meshlet_bg, &[]);
        pass.set_bind_group(2, &materials_bg, &[]);
        pass.set_bind_group(3, &scene_bg, &[]);
        pass.set_bind_group(5, lights_bg, &[]);
        for slot in slots {
            let refs = material_pipeline.slot_texture_refs(slot);
            let texture_bg = texture_pool.material_bind_group(device, refs[0], refs[1], refs[2]);
            let offset = (slot as u64 * self.screen_stride) as u32;
            pass.set_bind_group(0, &frame_bg, &[offset]);
            pass.set_bind_group(4, &texture_bg, &[]);
            pass.dispatch_workgroups(tiles_x, tiles_y, 1);
        }
    }
}

#[cfg(test)]
mod tests;
