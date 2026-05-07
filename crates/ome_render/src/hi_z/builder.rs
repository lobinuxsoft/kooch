//! `HiZ` struct: pipelines + mip views + dispatch logic.
//!
//! Production path: SPD (FidelityFX Single-Pass Downsampler) via
//! `cs_downsample_first` + `cs_downsample_second` in `hi_z_spd.wgsl`.
//! Two compute dispatches (wgpu lacks globally coherent storage
//! buffers needed for a true single-pass) but no per-mip
//! storage→texture transitions, which is what unblocks Mesa radv
//! (the per-mip approach this replaces, kept here for tests via
//! `build_from_r32`, hits driver bugs in long editor flows).
//!
//! Test/legacy path: `build_from_r32` keeps the `cs_copy_r32` +
//! `cs_reduce_max` per-mip pipelines from `hi_z_build.wgsl` for the
//! integration tests that pre-fill an R32Float source via
//! `Queue::write_texture` (forbidden on `Depth32Float`). This path
//! is NOT used in production and the dispatchers don't share state
//! with SPD.

use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::{mip_count_for, mip_size, HI_Z_FORMAT, SPD_PYRAMID_SLOT_COUNT, WORKGROUP_SIZE};

const SHADER_SOURCE: &str = include_str!("../../shaders/hi_z_build.wgsl");
const SPD_SHADER_SOURCE: &str = include_str!("../../shaders/hi_z_spd.wgsl");

/// Push-constant-equivalent uniform consumed by the SPD shader. The
/// second `_padN` words round the type to 16 bytes — wgpu requires
/// uniform binding sizes to be 16-byte multiples.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct SpdConstants {
    max_mip_level: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

pub struct HiZ {
    texture: wgpu::Texture,
    mip_views: Vec<wgpu::TextureView>,
    full_view: wgpu::TextureView,

    /// Virtual source dimensions used by SPD's `load_mip_0` to round
    /// non-pow2 source sizes up. Equals `(source + 1).next_power_of_two()`
    /// per Bevy's reference. Drives the first-dispatch workgroup
    /// count: `virtual / 64`.
    virtual_w: u32,
    virtual_h: u32,

    // ── SPD path (production) ──────────────────────────────────────
    spd_first_pipeline: wgpu::ComputePipeline,
    spd_second_pipeline: wgpu::ComputePipeline,
    spd_bgl: wgpu::BindGroupLayout,
    /// SPD's `max_mip_level` constants buffer. Written once at
    /// construction; never mutated per frame.
    spd_constants_buffer: wgpu::Buffer,
    spd_sampler: wgpu::Sampler,
    /// Dummy 1×1 R32Float texture for SPD slots beyond `mip_count`.
    /// Held here to keep the dummy view alive across frames.
    #[allow(dead_code)]
    spd_dummy_texture: wgpu::Texture,
    spd_dummy_view: wgpu::TextureView,

    // ── Legacy per-mip path (test-only) ────────────────────────────
    copy_r32_pipeline: wgpu::ComputePipeline,
    reduce_pipeline: wgpu::ComputePipeline,
    copy_r32_bgl: wgpu::BindGroupLayout,
    #[allow(dead_code)] // kept alive so cached `reduce_bgs` stay valid
    reduce_bgl: wgpu::BindGroupLayout,
    /// Pre-built bind groups for the legacy per-mip reduce path.
    /// Only used by `build_from_r32` (tests).
    reduce_bgs: Vec<wgpu::BindGroup>,

    width: u32,
    height: u32,
    mip_count: u32,
}

impl HiZ {
    /// Builds the pyramid resources for a depth attachment of size
    /// `(source_width, source_height)`. The pyramid texture itself
    /// is sized to `previous_power_of_two(source / 2)` because
    /// SPD's first downsample step writes pyramid mip 0 from a
    /// source 2× as wide; the rounding-down to prev-pow2 keeps the
    /// 2×2 reductions aligned across all mips. The cull shader's
    /// pixel-radius math reads `hi_z_size` from `HiZTestParams` and
    /// already tolerates the divergence between viewport and
    /// pyramid dimensions.
    ///
    /// Both source dimensions must be ≥ 2 so the pyramid has at
    /// least one mip.
    pub fn new(device: &wgpu::Device, source_width: u32, source_height: u32) -> Self {
        assert!(
            source_width >= 2 && source_height >= 2,
            "Hi-Z requires source dims ≥ 2 (got {source_width}×{source_height})"
        );

        // Match Bevy / SPD reference: virtual_view = next_power_of_two(source + 1),
        // pyramid mip 0 size = virtual / 2. The +1 forces the round
        // even when source is already a power of two so we don't lose
        // precision in the top mip.
        let virtual_w = (source_width + 1).next_power_of_two();
        let virtual_h = (source_height + 1).next_power_of_two();
        let width = (virtual_w / 2).max(1);
        let height = (virtual_h / 2).max(1);
        let mip_count = mip_count_for(width, height);

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hi_z_pyramid"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: mip_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HI_Z_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let mip_views: Vec<wgpu::TextureView> = (0..mip_count)
            .map(|mip| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("hi_z_mip_view"),
                    format: Some(HI_Z_FORMAT),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    usage: None,
                    aspect: wgpu::TextureAspect::All,
                    base_mip_level: mip,
                    mip_level_count: Some(1),
                    base_array_layer: 0,
                    array_layer_count: Some(1),
                })
            })
            .collect();

        let full_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("hi_z_full_view"),
            format: Some(HI_Z_FORMAT),
            dimension: Some(wgpu::TextureViewDimension::D2),
            usage: None,
            aspect: wgpu::TextureAspect::All,
            base_mip_level: 0,
            mip_level_count: None,
            base_array_layer: 0,
            array_layer_count: Some(1),
        });

        // ── SPD setup ──────────────────────────────────────────────
        let spd_dummy_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hi_z_spd_dummy"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HI_Z_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });
        let spd_dummy_view = spd_dummy_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("hi_z_spd_dummy_view"),
            format: Some(HI_Z_FORMAT),
            dimension: Some(wgpu::TextureViewDimension::D2),
            usage: None,
            aspect: wgpu::TextureAspect::All,
            base_mip_level: 0,
            mip_level_count: Some(1),
            base_array_layer: 0,
            array_layer_count: Some(1),
        });

        let spd_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("hi_z_spd_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let spd_constants = SpdConstants {
            max_mip_level: mip_count,
            _pad0: 0,
            _pad1: 0,
            _pad2: 0,
        };
        let spd_constants_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("hi_z_spd_constants"),
            contents: bytemuck::bytes_of(&spd_constants),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let spd_bgl = build_spd_bgl(device);
        let spd_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hi_z_spd_shader"),
            source: wgpu::ShaderSource::Wgsl(SPD_SHADER_SOURCE.into()),
        });
        let spd_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hi_z_spd_pipeline_layout"),
            bind_group_layouts: &[Some(&spd_bgl)],
            immediate_size: 0,
        });
        let spd_first_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("hi_z_spd_first_pipeline"),
            layout: Some(&spd_pipeline_layout),
            module: &spd_shader,
            entry_point: Some("cs_downsample_first"),
            compilation_options: Default::default(),
            cache: None,
        });
        let spd_second_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("hi_z_spd_second_pipeline"),
            layout: Some(&spd_pipeline_layout),
            module: &spd_shader,
            entry_point: Some("cs_downsample_second"),
            compilation_options: Default::default(),
            cache: None,
        });

        // mip_0 source binding is filled by `build_from_depth` per
        // call (the depth view changes between callers / frames).
        // Everything else is fixed for the lifetime of the HiZ
        // struct, so the bind group is built once. A second SPD
        // bind group with the depth view in slot 0 is built per
        // call; the rest of the slots ride along through
        // `with_mip_0_replaced`-style shadowing inside dispatch.
        // For simplicity we recreate the whole bind group per
        // build_from_depth (cheap — 14 slot references, no buffer
        // uploads). The result is parked in the orchestrator's
        // arena so it survives the queue.submit. See `build_from_depth`.

        // ── Legacy per-mip path (kept for tests) ───────────────────
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hi_z_build_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });
        let copy_r32_bgl = bgl_copy_r32(device);
        let reduce_bgl = bgl_reduce(device);

        let copy_r32_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hi_z_copy_r32_pipeline_layout"),
            bind_group_layouts: &[None, None, Some(&copy_r32_bgl)],
            immediate_size: 0,
        });
        let copy_r32_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("hi_z_copy_r32_pipeline"),
            layout: Some(&copy_r32_layout),
            module: &shader,
            entry_point: Some("cs_copy_r32"),
            compilation_options: Default::default(),
            cache: None,
        });

        let reduce_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hi_z_reduce_pipeline_layout"),
            bind_group_layouts: &[None, Some(&reduce_bgl)],
            immediate_size: 0,
        });
        let reduce_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("hi_z_reduce_pipeline"),
            layout: Some(&reduce_layout),
            module: &shader,
            entry_point: Some("cs_reduce_max"),
            compilation_options: Default::default(),
            cache: None,
        });

        let reduce_bgs: Vec<wgpu::BindGroup> = (1..mip_count)
            .map(|mip| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("hi_z_reduce_bg"),
                    layout: &reduce_bgl,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(
                                &mip_views[(mip - 1) as usize],
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(
                                &mip_views[mip as usize],
                            ),
                        },
                    ],
                })
            })
            .collect();

        Self {
            texture,
            mip_views,
            full_view,
            virtual_w,
            virtual_h,
            spd_first_pipeline,
            spd_second_pipeline,
            spd_bgl,
            spd_constants_buffer,
            spd_sampler,
            spd_dummy_texture,
            spd_dummy_view,
            copy_r32_pipeline,
            reduce_pipeline,
            copy_r32_bgl,
            reduce_bgl,
            reduce_bgs,
            width,
            height,
            mip_count,
        }
    }

    /// Records the SPD pyramid build into `encoder`. `depth_view`
    /// must reference a Depth32Float texture matching the dimensions
    /// passed to [`Self::new`].
    ///
    /// The `arena` is required: the caller MUST keep the per-call
    /// SPD bind group alive past `queue.submit`. wgpu does not
    /// internally Arc-clone bind groups on `set_bind_group`, and
    /// Mesa radv invalidates them if the local goes out of scope
    /// before the GPU reaches the dispatch. See PR #479 for the
    /// arena pattern this matches.
    pub fn build_from_depth(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        depth_view: &wgpu::TextureView,
        arena: &mut Vec<wgpu::BindGroup>,
    ) {
        let bg = build_spd_bind_group(
            device,
            &self.spd_bgl,
            depth_view,
            &self.mip_views,
            self.mip_count,
            &self.spd_dummy_view,
            &self.spd_sampler,
            &self.spd_constants_buffer,
        );

        // First dispatch: one workgroup per 64×64 virtual-source
        // tile. Each workgroup writes 32×32 pixels to mip_1
        // (= pyramid mip 0). `virtual_w/h` = source rounded up to
        // next pow2 (with +1 to force strict round); pyramid mip 0
        // size = virtual / 2, so workgroups = virtual / 64.
        let wg_x = (self.virtual_w / 64).max(1);
        let wg_y = (self.virtual_h / 64).max(1);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("hi_z_spd_first_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.spd_first_pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(wg_x, wg_y, 1);
        }

        // Second dispatch: one workgroup over mip 6 → mips 7..=12.
        // Skipped when the pyramid doesn't reach mip 7.
        if self.mip_count > 6 {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("hi_z_spd_second_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.spd_second_pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }

        arena.push(bg);
    }

    /// Test-only path: per-mip downsample from an R32Float source.
    /// Production callers should use [`Self::build_from_depth`].
    pub fn build_from_r32(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        r32_view: &wgpu::TextureView,
    ) {
        let copy_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hi_z_copy_r32_bg"),
            layout: &self.copy_r32_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(r32_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.mip_views[0]),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("hi_z_copy_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.copy_r32_pipeline);
            pass.set_bind_group(2, &copy_bg, &[]);
            pass.dispatch_workgroups(
                self.width.div_ceil(WORKGROUP_SIZE),
                self.height.div_ceil(WORKGROUP_SIZE),
                1,
            );
        }
        for mip in 1..self.mip_count {
            let (dst_w, dst_h) = mip_size(self.width, self.height, mip);
            let bg = &self.reduce_bgs[(mip - 1) as usize];
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("hi_z_reduce_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.reduce_pipeline);
            pass.set_bind_group(1, bg, &[]);
            pass.dispatch_workgroups(
                dst_w.div_ceil(WORKGROUP_SIZE),
                dst_h.div_ceil(WORKGROUP_SIZE),
                1,
            );
        }
    }

    /// Initialises the pyramid to "far" (1.0) by running SPD over a
    /// freshly-cleared depth view. The caller must clear the depth
    /// view to 1.0 in a separate submit before calling this — Mesa
    /// radv won't tolerate a depth-write → depth-sample transition
    /// inside a single encoder.
    pub fn init_to_far(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        cleared_depth_view: &wgpu::TextureView,
        arena: &mut Vec<wgpu::BindGroup>,
    ) {
        self.build_from_depth(device, encoder, cleared_depth_view, arena);
    }

    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub fn full_view(&self) -> &wgpu::TextureView {
        &self.full_view
    }

    pub fn mip_view(&self, mip: u32) -> &wgpu::TextureView {
        &self.mip_views[mip as usize]
    }

    pub fn mip_count(&self) -> u32 {
        self.mip_count
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn byte_size(&self) -> u64 {
        let mut total: u64 = 0;
        for level in 0..self.mip_count {
            let (w, h) = mip_size(self.width, self.height, level);
            total += (w as u64) * (h as u64) * 4;
        }
        total
    }

    pub fn pyramid_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hi_z_pyramid_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        })
    }
}

fn build_spd_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let mut entries = Vec::with_capacity(15);
    // mip_0: source depth attachment.
    entries.push(wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    });
    // mip_1..=mip_5: write-only storage.
    for binding in 1..=5 {
        entries.push(storage_slot(binding, wgpu::StorageTextureAccess::WriteOnly));
    }
    // mip_6: read_write (cross-workgroup sync between the two
    // dispatches). Requires `Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES`,
    // which the engine already enables in `gpu.rs::required_engine_features`.
    entries.push(storage_slot(6, wgpu::StorageTextureAccess::ReadWrite));
    // mip_7..=mip_12: write-only storage.
    for binding in 7..=12 {
        entries.push(storage_slot(binding, wgpu::StorageTextureAccess::WriteOnly));
    }
    // sampler for textureGather on mip_0.
    entries.push(wgpu::BindGroupLayoutEntry {
        binding: 13,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
        count: None,
    });
    // Constants UBO.
    entries.push(wgpu::BindGroupLayoutEntry {
        binding: 14,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: NonZeroU64::new(std::mem::size_of::<SpdConstants>() as u64),
        },
        count: None,
    });
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("hi_z_spd_bgl"),
        entries: &entries,
    })
}

fn storage_slot(binding: u32, access: wgpu::StorageTextureAccess) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access,
            format: HI_Z_FORMAT,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn build_spd_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    depth_view: &wgpu::TextureView,
    mip_views: &[wgpu::TextureView],
    mip_count: u32,
    dummy_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    constants: &wgpu::Buffer,
) -> wgpu::BindGroup {
    let mut entries = Vec::with_capacity(15);
    entries.push(wgpu::BindGroupEntry {
        binding: 0,
        resource: wgpu::BindingResource::TextureView(depth_view),
    });
    for slot in 0..SPD_PYRAMID_SLOT_COUNT as u32 {
        let view = if slot < mip_count {
            &mip_views[slot as usize]
        } else {
            dummy_view
        };
        entries.push(wgpu::BindGroupEntry {
            binding: slot + 1,
            resource: wgpu::BindingResource::TextureView(view),
        });
    }
    entries.push(wgpu::BindGroupEntry {
        binding: 13,
        resource: wgpu::BindingResource::Sampler(sampler),
    });
    entries.push(wgpu::BindGroupEntry {
        binding: 14,
        resource: constants.as_entire_binding(),
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("hi_z_spd_bg"),
        layout,
        entries: &entries,
    })
}

fn bgl_copy_r32(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("hi_z_copy_r32_bgl"),
        entries: &[float_src_entry(0), storage_dst_entry(1)],
    })
}

fn bgl_reduce(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("hi_z_reduce_bgl"),
        entries: &[float_src_entry(0), storage_dst_entry(1)],
    })
}

fn float_src_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn storage_dst_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: HI_Z_FORMAT,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_parses_and_validates() {
        let module =
            naga::front::wgsl::parse_str(SHADER_SOURCE).expect("hi_z_build.wgsl should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("hi_z_build.wgsl should validate");
    }

    #[test]
    fn spd_shader_parses_and_validates() {
        let module = naga::front::wgsl::parse_str(SPD_SHADER_SOURCE)
            .expect("hi_z_spd.wgsl should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("hi_z_spd.wgsl should validate");
    }
}
