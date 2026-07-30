use wgpu::util::DeviceExt;

use super::super::{HI_Z_FORMAT, mip_count_for, mip_size};
use super::legacy::{bgl_copy_r32, bgl_reduce};
use super::spd::build_spd_bgl;
use super::types::{HiZ, SpdConstants};
use super::{SHADER_SOURCE, SPD_SHADER_SOURCE};

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
        let spd_second_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
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
                            resource: wgpu::BindingResource::TextureView(&mip_views[mip as usize]),
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
