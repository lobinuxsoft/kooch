//! `HiZ` struct: pipeline + mip views + dispatch logic.

use super::{mip_count_for, mip_size, HI_Z_FORMAT, WORKGROUP_SIZE};

const SHADER_SOURCE: &str = include_str!("../../shaders/hi_z_build.wgsl");

/// Owns the Hi-Z pyramid texture, the per-mip writeable views, the
/// three compute pipelines (`copy_depth`, `copy_r32`, `reduce`), and
/// the bind-group layouts. Re-create when the viewport resizes.
pub struct HiZ {
    texture: wgpu::Texture,
    mip_views: Vec<wgpu::TextureView>,
    full_view: wgpu::TextureView,

    copy_depth_pipeline: wgpu::ComputePipeline,
    copy_r32_pipeline: wgpu::ComputePipeline,
    reduce_pipeline: wgpu::ComputePipeline,
    copy_depth_bgl: wgpu::BindGroupLayout,
    copy_r32_bgl: wgpu::BindGroupLayout,
    reduce_bgl: wgpu::BindGroupLayout,

    /// Pre-built bind groups for the per-mip reduction passes. One
    /// per (src_mip, dst_mip) pair, indexed by `dst_mip - 1`. Cached
    /// at construction so they outlive any single frame's submit —
    /// wgpu does not internally Arc-clone bind groups on
    /// `set_bind_group`, so creating them inline per dispatch and
    /// dropping the locals between record and submit invalidates the
    /// view references on Mesa radv (RX 9070 XT).
    reduce_bgs: Vec<wgpu::BindGroup>,

    width: u32,
    height: u32,
    mip_count: u32,
}

impl HiZ {
    /// Builds the pyramid resources for an `(width, height)` depth
    /// target. Both must be ≥ 1.
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        assert!(width > 0 && height > 0, "Hi-Z requires non-zero dimensions");

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

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hi_z_build_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let copy_depth_bgl = bgl_copy_depth(device);
        let copy_r32_bgl = bgl_copy_r32(device);
        let reduce_bgl = bgl_reduce(device);

        let copy_depth_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hi_z_copy_depth_pipeline_layout"),
            bind_group_layouts: &[Some(&copy_depth_bgl)],
            immediate_size: 0,
        });
        let copy_depth_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("hi_z_copy_depth_pipeline"),
                layout: Some(&copy_depth_layout),
                module: &shader,
                entry_point: Some("cs_copy_depth"),
                compilation_options: Default::default(),
                cache: None,
            });

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

        // Pre-build the per-mip reduce bind groups so they live as
        // long as the HiZ struct. Inline creation in dispatch_reductions
        // had the bind groups going out of scope between encoder
        // record and queue.submit, which Mesa radv (RX 9070 XT)
        // surfaces as "TextureView is invalid" at submit time.
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
            copy_depth_pipeline,
            copy_r32_pipeline,
            reduce_pipeline,
            copy_depth_bgl,
            copy_r32_bgl,
            reduce_bgl,
            reduce_bgs,
            width,
            height,
            mip_count,
        }
    }

    /// Records the entire pyramid build into `encoder`. `depth_view`
    /// must reference a Depth32Float texture matching the dimensions
    /// passed to [`Self::new`].
    ///
    /// The `arena` is an optional out-param the caller passes when it
    /// needs to keep the per-frame `copy_bg` alive past the call.
    /// wgpu does not internally Arc-clone bind groups on
    /// `set_bind_group`, so a bind group created inline + dropped
    /// before `queue.submit` ends up "invalid" on Mesa radv (RX 9070
    /// XT). Pass `Some(&mut arena)` from the orchestrator and clear
    /// the arena after the submit. `None` is fine for tests where
    /// the call site already keeps the encoder + submit + drop tight
    /// enough that the wgpu driver hasn't lost the reference yet.
    pub fn build_from_depth(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        depth_view: &wgpu::TextureView,
        arena: Option<&mut Vec<wgpu::BindGroup>>,
    ) {
        let copy_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hi_z_copy_depth_bg"),
            layout: &self.copy_depth_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.mip_views[0]),
                },
            ],
        });
        self.dispatch_copy(encoder, &self.copy_depth_pipeline, &copy_bg, 0);
        self.dispatch_reductions(device, encoder);
        if let Some(arena) = arena {
            arena.push(copy_bg);
        }
    }

    /// Same as [`Self::build_from_depth`] but takes an `R32Float`
    /// source texture instead of a depth attachment. Used by tests
    /// because wgpu forbids `Queue::write_texture` writes to
    /// `Depth32Float`; production code paths should call
    /// [`Self::build_from_depth`].
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
        self.dispatch_copy(encoder, &self.copy_r32_pipeline, &copy_bg, 2);
        self.dispatch_reductions(device, encoder);
    }

    fn dispatch_copy(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        bind_group: &wgpu::BindGroup,
        bind_slot: u32,
    ) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("hi_z_copy_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(bind_slot, bind_group, &[]);
        pass.dispatch_workgroups(
            self.width.div_ceil(WORKGROUP_SIZE),
            self.height.div_ceil(WORKGROUP_SIZE),
            1,
        );
    }

    fn dispatch_reductions(
        &self,
        _device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
    ) {
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

    /// Pyramid texture (R32Float, all mips). Occlusion culling reads
    /// it through [`Self::full_view`].
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    /// Multi-mip view suitable for `textureLoad(hi_z, coord, lod)` in
    /// the cull shader.
    pub fn full_view(&self) -> &wgpu::TextureView {
        &self.full_view
    }

    /// Single-mip view at `mip`. Useful for tests / debug overlays.
    pub fn mip_view(&self, mip: u32) -> &wgpu::TextureView {
        &self.mip_views[mip as usize]
    }

    pub fn mip_count(&self) -> u32 {
        self.mip_count
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Approximate persistent VRAM footprint of the pyramid texture
    /// across all mips. R32Float = 4 bytes per pixel, summed over the
    /// `mip_count_for(width, height)` mip chain. Used by the engine
    /// VRAM tracker (#463.5) when the render stage owns the pyramid.
    pub fn byte_size(&self) -> u64 {
        let mut total: u64 = 0;
        for level in 0..self.mip_count {
            let (w, h) = mip_size(self.width, self.height, level);
            total += (w as u64) * (h as u64) * 4;
        }
        total
    }

    /// Initialises the pyramid to 1.0 (the "far" value the
    /// conservative Hi-Z reject test treats as "nothing occluded")
    /// using the existing build pipeline — `cs_copy_depth` from a
    /// freshly-cleared depth view writes 1.0 to mip 0, then
    /// `cs_reduce_max` propagates that across the chain. Required on
    /// the first-ever frame using the 2-pass cull (#445), since the
    /// previous-frame pyramid has no real depth data and undefined
    /// R32Float bytes would otherwise read as "everything at depth
    /// 0" and reject every meshlet.
    ///
    /// `cleared_depth_view` must be a depth-only view of a
    /// Depth32Float texture whose contents are 1.0 everywhere — the
    /// caller arranges that with a one-shot render pass using
    /// `LoadOp::Clear(1.0)` and no draws into the same encoder
    /// before this call.
    ///
    /// Done via the encoder rather than a `Queue::write_texture`
    /// because the latter triggers wgpu validation churn around
    /// `STORAGE_BINDING + TEXTURE_BINDING` view aliasing on Mesa
    /// (RX 9070 XT radv) — the views land in an "invalid" state for
    /// the next bind-group construction. The build path is the
    /// known-good code path; reusing it costs one render-pass clear
    /// + the standard pyramid build, both already in the hot path.
    pub fn init_to_far(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        cleared_depth_view: &wgpu::TextureView,
        arena: Option<&mut Vec<wgpu::BindGroup>>,
    ) {
        self.build_from_depth(device, encoder, cleared_depth_view, arena);
    }

    /// Bind-group layout the cull shader (PR-5c) consumes when
    /// sampling Hi-Z. Caller assembles a bind group with `full_view`
    /// at `binding(0)`.
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

fn bgl_copy_depth(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("hi_z_copy_depth_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            storage_dst_entry(1),
        ],
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
}
